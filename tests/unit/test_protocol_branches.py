from __future__ import annotations

import sys
from collections.abc import Mapping, Sequence
from datetime import UTC, datetime
from io import TextIOBase
from pathlib import Path
from typing import Any, cast

import pytest

from kala_agent.contracts import ArtifactClassification, ArtifactRef, ToolRunStatus
from kala_agent.tools import protocol as protocol_module
from kala_agent.tools.protocol import (
    MAX_COLLECTION_ITEMS,
    MAX_JSON_BYTES,
    MAX_KEY_LENGTH,
    MAX_PATH_SEGMENTS,
    MAX_PATTERN_INPUT_LENGTH,
    MAX_PATTERN_LENGTH,
    MAX_PATTERN_REPEAT,
    MAX_STRING_LENGTH,
    PROTOCOL_VERSION,
    JsonEnvelopeError,
    JsonObject,
    SchemaValidationError,
    SubprocessToolHandler,
    ToolEnvelopeError,
    ToolEnvelopeKind,
    ToolErrorCode,
    ToolRequestEnvelope,
    ToolResponseEnvelope,
    ToolWorkspaceError,
    _bounded_identifier,
    _bounded_pattern,
    _bounded_text,
    _bounded_version,
    _capability_tuple,
    _frozen_object,
    _json_equal,
    _normalize_json,
    _positive_integer,
    _positive_number,
    _quantifier_at,
    _reject_nested_repetition,
    _run_status,
    _schema_branch,
    _schema_float,
    _schema_int,
    _schema_keywords,
    build_child_environment,
    build_worker_command,
    canonical_json_digest,
    canonical_json_dumps,
    check_object_schema,
    read_request_stream,
    resolve_workspace_path,
    sanitize_child_stderr,
    sha256_hex,
    strict_json_loads,
    summarize_error,
    validate_against_schema,
    write_response_stream,
)

STARTED_AT = datetime(2026, 1, 2, 3, 4, 5, tzinfo=UTC)
FINISHED_AT = datetime(2026, 1, 2, 3, 4, 6, tzinfo=UTC)


def _artifact(artifact_id: str = "artifact-1") -> ArtifactRef:
    return ArtifactRef(
        artifact_id=artifact_id,
        uri="file:///tmp/report.json",
        digest=sha256_hex(b"payload"),
        media_type="application/json",
        size_bytes=7,
        classification=ArtifactClassification.TEXT,
    )


def _request(**overrides: object) -> ToolRequestEnvelope:
    values: dict[str, Any] = {
        "protocol_version": PROTOCOL_VERSION,
        "kind": ToolEnvelopeKind.REQUEST,
        "call_id": "call-1",
        "tool_id": "mechanical.reference-simulator",
        "tool_version": "1.0.0",
        "arguments": cast("dict[str, Any]", {"operation": "beam_volume"}),
        "workspace": "/tmp/workspace",
        "deadline_seconds": 30.0,
        "max_output_bytes": 1_000_000,
        "max_artifact_bytes": 1_000_000,
        "granted_capabilities": (),
    }
    values.update(cast("Mapping[str, Any]", overrides))
    return ToolRequestEnvelope(**values)


def _succeeded(**overrides: object) -> ToolResponseEnvelope:
    values: dict[str, Any] = {
        "call_id": "call-1",
        "tool_id": "mechanical.reference-simulator",
        "tool_version": "1.0.0",
        "result": cast("dict[str, Any]", {"ok": True}),
    }
    values.update(cast("Mapping[str, Any]", overrides))
    return ToolResponseEnvelope.succeeded_envelope(**values)


def _failed(**overrides: object) -> ToolResponseEnvelope:
    values: dict[str, Any] = {
        "call_id": "call-1",
        "tool_id": "mechanical.reference-simulator",
        "tool_version": "1.0.0",
        "error_code": ToolErrorCode.INTERNAL_ERROR,
        "error_message": "the tool failed",
    }
    values.update(cast("Mapping[str, Any]", overrides))
    return ToolResponseEnvelope.failed_envelope(**values)


def test_json_equal_distinguishes_booleans_from_numbers() -> None:
    assert _json_equal(1, True) is False
    assert _json_equal(True, 1) is False
    assert _json_equal(True, True) is True


def test_json_equal_compares_mappings_recursively() -> None:
    assert _json_equal({"a": 1, "b": 2}, {"b": 2, "a": 1}) is True
    assert _json_equal({"a": 1}, {"a": 1, "b": 2}) is False
    assert _json_equal({"a": {"b": 1}}, {"a": {"b": 2}}) is False


def test_json_equal_compares_sequences_positionally() -> None:
    assert _json_equal([1, 2], (1, 2)) is True
    assert _json_equal([1, 2], [1, 2, 3]) is False
    assert _json_equal([1, 2], [1, 3]) is False


def test_normalize_json_rejects_excessive_depth() -> None:
    nested: Any = "leaf"
    for _ in range(MAX_STRING_LENGTH):
        nested = {"a": nested}
    with pytest.raises(JsonEnvelopeError, match="nesting exceeds"):
        _normalize_json(nested, depth=0)


def test_normalize_json_rejects_oversized_strings() -> None:
    with pytest.raises(JsonEnvelopeError, match="string exceeds"):
        _normalize_json("x" * (MAX_STRING_LENGTH + 1), depth=0)
    assert _normalize_json("x", depth=0) == "x"


def test_normalize_json_rejects_non_finite_floats() -> None:
    with pytest.raises(JsonEnvelopeError, match="must be finite"):
        _normalize_json(float("inf"), depth=0)
    assert _normalize_json(1.5, depth=0) == 1.5


def test_normalize_json_rejects_non_string_keys() -> None:
    with pytest.raises(JsonEnvelopeError, match="keys must be strings"):
        _normalize_json({7: "value"}, depth=0)


def test_normalize_json_rejects_oversized_keys() -> None:
    with pytest.raises(JsonEnvelopeError, match="key exceeds"):
        _normalize_json({"k" * (MAX_KEY_LENGTH + 1): 1}, depth=0)


def test_normalize_json_rejects_oversized_collections() -> None:
    with pytest.raises(JsonEnvelopeError, match="bounded item count"):
        _normalize_json([0] * (MAX_COLLECTION_ITEMS + 1), depth=0)


def test_normalize_json_rejects_unsupported_values() -> None:
    with pytest.raises(JsonEnvelopeError, match="not JSON serializable"):
        _normalize_json({1, 2}, depth=0)
    with pytest.raises(JsonEnvelopeError, match="not JSON serializable"):
        _normalize_json(object(), depth=0)


def test_canonical_json_dumps_sorts_and_escapes() -> None:
    assert canonical_json_dumps({"b": 1, "a": "é"}) == '{"a":"\\u00e9","b":1}'
    assert canonical_json_dumps((1, 2)) == "[1,2]"


def test_canonical_json_dumps_enforces_the_byte_budget() -> None:
    with pytest.raises(JsonEnvelopeError, match="bounded size"):
        canonical_json_dumps("x" * 100, max_bytes=10)


def test_canonical_json_dumps_reports_unserializable_values() -> None:
    with pytest.raises(JsonEnvelopeError, match="not JSON serializable"):
        canonical_json_dumps(cast("dict[str, Any]", {"a": {1, 2}}))


def test_canonical_json_dumps_reports_recursion_failures(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    import json as json_module

    def explode(*args: object, **kwargs: object) -> str:
        raise RecursionError("too deep")

    monkeypatch.setattr(json_module, "dumps", explode)
    with pytest.raises(JsonEnvelopeError, match="cannot be canonicalized"):
        canonical_json_dumps({"a": 1})


def test_strict_json_loads_rejects_unsupported_payload_types() -> None:
    with pytest.raises(TypeError, match="str or bytes"):
        strict_json_loads(cast("str | bytes", 7))


def test_strict_json_loads_enforces_the_byte_budget() -> None:
    with pytest.raises(JsonEnvelopeError, match="bounded size"):
        strict_json_loads(b'"xxxx"', max_bytes=2)
    assert strict_json_loads(b'{"a":1}') == {"a": 1}
    assert strict_json_loads(bytes(bytearray(b"[1]"))) == [1]


def test_strict_json_loads_rejects_invalid_utf8() -> None:
    with pytest.raises(JsonEnvelopeError, match="not valid UTF-8"):
        strict_json_loads(b'{"a":"\xff\xfe"}')


def test_strict_json_loads_rejects_json_constants() -> None:
    for literal in ("NaN", "Infinity", "-Infinity"):
        with pytest.raises(JsonEnvelopeError, match="not permitted"):
            strict_json_loads(literal)


def test_strict_json_loads_rejects_non_finite_floats() -> None:
    with pytest.raises(JsonEnvelopeError, match="must be finite"):
        strict_json_loads("1e400")


def test_strict_json_loads_rejects_duplicate_keys() -> None:
    with pytest.raises(JsonEnvelopeError, match="duplicate JSON object keys"):
        strict_json_loads('{"a":1,"a":2}')


def test_strict_json_loads_reports_malformed_payloads() -> None:
    with pytest.raises(JsonEnvelopeError, match="not strict JSON"):
        strict_json_loads("{not json")
    with pytest.raises(JsonEnvelopeError, match="not strict JSON"):
        strict_json_loads("[" * 10_000)


def test_sha256_hex_accepts_only_text_or_bytes() -> None:
    assert sha256_hex("payload") == sha256_hex(b"payload")
    assert sha256_hex(bytes(bytearray(b"payload"))) == sha256_hex(b"payload")
    with pytest.raises(TypeError, match="str or bytes"):
        sha256_hex(cast("str | bytes", 7))


def test_canonical_json_digest_is_stable_across_key_order() -> None:
    assert canonical_json_digest({"a": 1, "b": 2}) == canonical_json_digest({"b": 2, "a": 1})


def test_schema_keywords_rejects_unsupported_names() -> None:
    with pytest.raises(SchemaValidationError, match="unsupported schema keyword"):
        _schema_keywords({"nonsense": 1}, path="$")
    _schema_keywords({"title": "x", "type": "object"}, path="$")


def test_schema_keywords_rejects_non_string_keywords() -> None:
    with pytest.raises(SchemaValidationError, match="must be strings"):
        _schema_keywords(cast("Mapping[str, object]", {7: 1}), path="$")


def test_schema_int_and_float_bounds() -> None:
    assert _schema_int({}, "minimum", path="$") is None
    assert _schema_int({"minimum": 3}, "minimum", path="$") == 3
    assert _schema_float({"maximum": 2}, "maximum", path="$") == 2.0
    with pytest.raises(SchemaValidationError, match="must be a number"):
        _schema_int({"minimum": "3"}, "minimum", path="$")
    with pytest.raises(SchemaValidationError, match="must be finite"):
        _schema_float({"maximum": float("inf")}, "maximum", path="$")
    with pytest.raises(SchemaValidationError, match="must be an integer"):
        _schema_int({"minimum": 1.5}, "minimum", path="$")
    with pytest.raises(SchemaValidationError, match="must be a number"):
        _schema_int({"minimum": True}, "minimum", path="$")


def test_schema_branch_requires_a_mapping() -> None:
    with pytest.raises(SchemaValidationError, match="branches must be objects"):
        _schema_branch(1, cast(Mapping[str, object], "nope"), path="$", depth=0)


def test_schema_branch_reports_mismatch_as_false() -> None:
    assert _schema_branch(1, {"type": "integer"}, path="$", depth=0) is True
    assert _schema_branch("x", {"type": "integer"}, path="$", depth=0) is False


def test_validate_against_schema_rejects_non_mapping_schemas() -> None:
    with pytest.raises(SchemaValidationError, match="schema must be an object"):
        validate_against_schema(1, cast(Mapping[str, object], ["type"]))


def _nested_all_of(depth: int) -> dict[str, Any]:
    schema: dict[str, Any] = {"type": "integer"}
    for _ in range(depth):
        schema = {"allOf": [schema]}
    return schema


def test_validate_against_schema_rejects_excessive_depth() -> None:
    with pytest.raises(SchemaValidationError, match="nesting exceeds"):
        validate_against_schema(1, {"type": "integer"}, depth=40)


def test_validate_against_schema_swallows_deep_branch_failures() -> None:
    with pytest.raises(SchemaValidationError, match="required subschema"):
        validate_against_schema(1, _nested_all_of(40))


def test_validate_against_schema_accepts_nesting_within_the_bound() -> None:
    validate_against_schema(1, _nested_all_of(20))
    validate_against_schema(
        {"a": {"b": 1}},
        {
            "type": "object",
            "properties": {
                "a": {
                    "type": "object",
                    "properties": {"b": {"type": "integer"}},
                }
            },
        },
    )


def test_validate_against_schema_accepts_union_types() -> None:
    validate_against_schema("x", {"type": ["string", "null"]})
    with pytest.raises(SchemaValidationError, match="does not match the declared type"):
        validate_against_schema(1, {"type": ["string", "null"]})


def test_validate_against_schema_rejects_unknown_types() -> None:
    with pytest.raises(SchemaValidationError, match="unsupported schema type"):
        validate_against_schema(1, {"type": "date"})
    with pytest.raises(SchemaValidationError, match="unsupported schema type"):
        validate_against_schema(1, {"type": [7]})


def test_validate_against_schema_enforces_const() -> None:
    validate_against_schema(1, {"const": 1})
    validate_against_schema(True, {"const": True})
    with pytest.raises(SchemaValidationError, match="declared const"):
        validate_against_schema(1, {"const": True})
    with pytest.raises(SchemaValidationError, match="declared const"):
        validate_against_schema(2, {"const": 1})


def test_json_equal_keeps_numbers_and_booleans_apart() -> None:
    assert _json_equal(1, 1.0) is True
    assert _json_equal({"a": 1}, {"a": True}) is False


def test_validate_against_schema_enforces_enum() -> None:
    validate_against_schema("a", {"enum": ["a", "b"]})
    with pytest.raises(SchemaValidationError, match="nonempty array"):
        validate_against_schema("a", {"enum": []})
    with pytest.raises(SchemaValidationError, match="nonempty array"):
        validate_against_schema("a", {"enum": "a"})
    with pytest.raises(SchemaValidationError, match="permitted enum member"):
        validate_against_schema("c", {"enum": ["a", "b"]})


def test_validate_against_schema_enforces_all_of() -> None:
    schema = {"allOf": [{"type": "integer"}, {"minimum": 1}]}
    validate_against_schema(2, schema)
    with pytest.raises(SchemaValidationError, match="required subschema"):
        validate_against_schema(0, schema)
    with pytest.raises(SchemaValidationError, match="allOf must be a nonempty array"):
        validate_against_schema(1, {"allOf": []})


def test_validate_against_schema_enforces_any_of() -> None:
    validate_against_schema(1, {"anyOf": [{"type": "string"}, {"type": "integer"}]})
    with pytest.raises(SchemaValidationError, match="anyOf subschema"):
        validate_against_schema(None, {"anyOf": [{"type": "string"}]})
    with pytest.raises(SchemaValidationError, match="anyOf must be a nonempty array"):
        validate_against_schema(1, {"anyOf": "nope"})


def test_validate_against_schema_enforces_one_of_exclusivity() -> None:
    schema = {"oneOf": [{"type": "integer"}, {"minimum": 0}]}
    validate_against_schema("x", {"oneOf": [{"type": "string"}]})
    with pytest.raises(SchemaValidationError, match="more than one oneOf"):
        validate_against_schema(1, schema)
    with pytest.raises(SchemaValidationError, match="oneOf must be a nonempty array"):
        validate_against_schema(1, {"oneOf": []})


def test_object_schema_rejects_malformed_properties() -> None:
    with pytest.raises(SchemaValidationError, match="properties must be an object"):
        validate_against_schema({}, {"properties": "nope"})
    with pytest.raises(SchemaValidationError, match="map names to schemas"):
        validate_against_schema({}, {"properties": {"a": "nope"}})
    with pytest.raises(SchemaValidationError, match="map names to schemas"):
        validate_against_schema({}, cast("Mapping[str, object]", {"properties": {7: {}}}))


def test_object_schema_rejects_malformed_required() -> None:
    with pytest.raises(SchemaValidationError, match="required must be an array"):
        validate_against_schema({}, {"required": "a"})
    with pytest.raises(SchemaValidationError, match="required entries must be strings"):
        validate_against_schema({}, {"required": [7]})
    with pytest.raises(SchemaValidationError, match="required entries must be unique"):
        validate_against_schema({}, {"required": ["a", "a"]})
    with pytest.raises(SchemaValidationError, match="is not declared"):
        validate_against_schema({}, {"required": ["a"], "properties": {"b": {}}})
    with pytest.raises(SchemaValidationError, match="is missing"):
        validate_against_schema({}, {"required": ["a"], "properties": {"a": {}}})


def test_object_schema_enforces_property_counts() -> None:
    with pytest.raises(SchemaValidationError, match="too few properties"):
        validate_against_schema({}, {"minProperties": 1})
    with pytest.raises(SchemaValidationError, match="too many properties"):
        validate_against_schema({"a": 1, "b": 2}, {"maxProperties": 1})


def test_object_schema_rejects_non_string_keys() -> None:
    with pytest.raises(SchemaValidationError, match="object keys must be strings"):
        validate_against_schema(cast("dict[object, object]", {7: 1}), {"type": "object"})


def test_object_schema_rejects_unpermitted_properties() -> None:
    with pytest.raises(SchemaValidationError, match="unpermitted properties"):
        validate_against_schema({"a": 1, "z": 2}, {"additionalProperties": False})


def test_object_schema_applies_additional_properties_schema() -> None:
    validate_against_schema({"a": "x"}, {"additionalProperties": {"type": "string"}})
    with pytest.raises(SchemaValidationError, match="does not match the declared type"):
        validate_against_schema({"a": 1}, {"additionalProperties": {"type": "string"}})


def test_object_schema_rejects_bad_additional_properties() -> None:
    with pytest.raises(SchemaValidationError, match="boolean or schema"):
        validate_against_schema({}, {"additionalProperties": "nope"})


def test_array_schema_enforces_item_counts() -> None:
    with pytest.raises(SchemaValidationError, match="bounded item count"):
        validate_against_schema([0] * (MAX_COLLECTION_ITEMS + 1), {"type": "array"})
    with pytest.raises(SchemaValidationError, match="too few items"):
        validate_against_schema([1], {"minItems": 2})
    with pytest.raises(SchemaValidationError, match="too many items"):
        validate_against_schema([1, 2], {"maxItems": 1})


def test_array_schema_rejects_bad_items_keyword() -> None:
    with pytest.raises(SchemaValidationError, match="items must be a schema"):
        validate_against_schema([1], {"items": "nope"})


def test_array_schema_reports_the_failing_index() -> None:
    with pytest.raises(SchemaValidationError) as error:
        validate_against_schema([1, "x"], {"items": {"type": "integer"}})
    assert error.value.path == "$[1]"


def test_array_schema_enforces_uniqueness() -> None:
    validate_against_schema([1, 2], {"uniqueItems": True})
    with pytest.raises(SchemaValidationError, match="must be unique"):
        validate_against_schema([{"a": 1}, {"a": 1}], {"uniqueItems": True})


def test_string_schema_enforces_lengths() -> None:
    with pytest.raises(SchemaValidationError, match="shorter than"):
        validate_against_schema("ab", {"minLength": 3})
    with pytest.raises(SchemaValidationError, match="longer than"):
        validate_against_schema("abcd", {"maxLength": 3})


def test_string_schema_enforces_patterns() -> None:
    validate_against_schema("abc", {"pattern": "^[a-z]+$", "maxLength": 8})
    with pytest.raises(SchemaValidationError, match="declared pattern"):
        validate_against_schema("ABC", {"pattern": "^[a-z]+$", "maxLength": 8})


def test_string_schema_rejects_non_string_patterns() -> None:
    with pytest.raises(SchemaValidationError, match="pattern must be a string"):
        validate_against_schema("abc", {"pattern": 7})


def test_number_schema_enforces_bounds() -> None:
    validate_against_schema(5, {"minimum": 0, "maximum": 10})
    with pytest.raises(SchemaValidationError, match="below the permitted minimum"):
        validate_against_schema(-1, {"minimum": 0})
    with pytest.raises(SchemaValidationError, match="above the permitted maximum"):
        validate_against_schema(11, {"maximum": 10})
    with pytest.raises(SchemaValidationError, match="not above the exclusive minimum"):
        validate_against_schema(0, {"exclusiveMinimum": 0})
    with pytest.raises(SchemaValidationError, match="not below the exclusive maximum"):
        validate_against_schema(10, {"exclusiveMaximum": 10})


def test_number_schema_enforces_multiple_of() -> None:
    validate_against_schema(6, {"multipleOf": 3})
    with pytest.raises(SchemaValidationError, match="not a multiple"):
        validate_against_schema(7, {"multipleOf": 3})
    with pytest.raises(SchemaValidationError, match="multipleOf must be positive"):
        validate_against_schema(7, {"multipleOf": 0})


def test_bounded_pattern_requires_a_max_length() -> None:
    with pytest.raises(SchemaValidationError, match="requires a maxLength"):
        _bounded_pattern("^a$", maximum_length=None, path="$")
    with pytest.raises(SchemaValidationError, match="between 1 and"):
        _bounded_pattern("^a$", maximum_length=0, path="$")
    with pytest.raises(SchemaValidationError, match="between 1 and"):
        _bounded_pattern("^a$", maximum_length=MAX_PATTERN_INPUT_LENGTH + 1, path="$")


def test_bounded_pattern_rejects_oversized_patterns() -> None:
    with pytest.raises(SchemaValidationError, match="bounded length"):
        _bounded_pattern("a" * (MAX_PATTERN_LENGTH + 1), maximum_length=32, path="$")


def test_bounded_pattern_rejects_every_unsafe_construct() -> None:
    unsafe = (
        r"(a)\1",
        r"(?=a)",
        r"(?!a)",
        r"(?<=a)",
        r"(?<!a)",
        r"(?(1)yes|no)",
        r"(?i-m)",
        r"(?>a)",
    )
    for pattern in unsafe:
        with pytest.raises(SchemaValidationError, match="unsafe construct"):
            _bounded_pattern(pattern, maximum_length=32, path="$")


def test_bounded_pattern_rejects_excessive_repetition() -> None:
    with pytest.raises(SchemaValidationError, match="repetition exceeds"):
        _bounded_pattern(f"a{{{MAX_PATTERN_REPEAT + 1}}}", maximum_length=32, path="$")


def test_bounded_pattern_rejects_invalid_regular_expressions() -> None:
    for pattern in (r"[a-", r"(?P<>a)", r"(?P<n>a)(?P<n>b)"):
        with pytest.raises(SchemaValidationError, match="not a valid regular expression"):
            _bounded_pattern(pattern, maximum_length=32, path="$")


def test_bounded_pattern_rejects_wildcard_repetition() -> None:
    for pattern in (r".*", r".+", r".{2,}", r". *"):
        with pytest.raises(SchemaValidationError, match="unsafe construct"):
            _bounded_pattern(pattern, maximum_length=32, path="$")


def test_bounded_pattern_accepts_single_level_repetition() -> None:
    assert _bounded_pattern("(ab)+", maximum_length=32, path="$") is not None
    assert _bounded_pattern("(a+)$", maximum_length=32, path="$") is not None
    assert _bounded_pattern(r"a\{2,4\}", maximum_length=32, path="$") is not None


def test_reject_nested_repetition_detects_unbalanced_groups() -> None:
    with pytest.raises(SchemaValidationError, match="unbalanced group"):
        _reject_nested_repetition("(a", path="$")
    with pytest.raises(SchemaValidationError, match="unbalanced group"):
        _reject_nested_repetition("a)", path="$")


def test_reject_nested_repetition_rejects_quantified_quantified_groups() -> None:
    with pytest.raises(SchemaValidationError, match="already contains a quantifier"):
        _reject_nested_repetition("(a+)+", path="$")
    with pytest.raises(SchemaValidationError, match="already contains a quantifier"):
        _reject_nested_repetition("(?:a{1,2})*", path="$")


def test_reject_nested_repetition_ignores_escapes_and_classes() -> None:
    _reject_nested_repetition(r"\[a\]+\(", path="$")
    _reject_nested_repetition(r"[\+*]+", path="$")
    _reject_nested_repetition(r"\(a\)+", path="$")


def test_reject_nested_repetition_ignores_lazy_quantifiers() -> None:
    _reject_nested_repetition("(a+?)", path="$")
    _reject_nested_repetition("(?:a?)", path="$")


def test_quantifier_at_recognizes_every_quantifier() -> None:
    assert _quantifier_at("a*", 1) == 2
    assert _quantifier_at("a+", 1) == 2
    assert _quantifier_at("a?", 1) == 2
    assert _quantifier_at("a{2}", 1) == 4
    assert _quantifier_at("a{2,4}", 1) == 6
    assert _quantifier_at("ab", 1) is None
    assert _quantifier_at("a{not a repeat}", 1) is None


def test_check_object_schema_requires_an_object_type() -> None:
    with pytest.raises(SchemaValidationError, match="must be a JSON object"):
        check_object_schema(["type"])
    with pytest.raises(SchemaValidationError, match="must declare an object type"):
        check_object_schema({"type": "string"})


def test_check_object_schema_requires_closed_properties() -> None:
    with pytest.raises(SchemaValidationError, match="forbid additional properties"):
        check_object_schema({"type": "object", "properties": {"a": {}}})
    with pytest.raises(SchemaValidationError, match="forbid additional properties"):
        check_object_schema(
            {"type": "object", "additionalProperties": True, "properties": {"a": {}}}
        )


def test_check_object_schema_requires_a_nonempty_properties_map() -> None:
    with pytest.raises(SchemaValidationError, match="nonempty properties map"):
        check_object_schema({"type": "object", "additionalProperties": False})
    with pytest.raises(SchemaValidationError, match="map names to schemas"):
        check_object_schema(
            {"type": "object", "additionalProperties": False, "properties": {"a": "nope"}}
        )
    with pytest.raises(SchemaValidationError, match="map names to schemas"):
        check_object_schema(
            {
                "type": "object",
                "additionalProperties": False,
                "properties": cast("Mapping[str, object]", {7: {}}),
            }
        )


def test_check_object_schema_validates_property_subschemas() -> None:
    with pytest.raises(SchemaValidationError, match="unsupported schema keyword"):
        check_object_schema(
            {
                "type": "object",
                "additionalProperties": False,
                "properties": {"a": {"nonsense": 1}},
            }
        )


def test_check_object_schema_validates_required_entries() -> None:
    with pytest.raises(SchemaValidationError, match="required entries must be an array"):
        check_object_schema(
            {
                "type": "object",
                "additionalProperties": False,
                "properties": {"a": {}},
                "required": "a",
            }
        )
    with pytest.raises(SchemaValidationError, match="undeclared property"):
        check_object_schema(
            {
                "type": "object",
                "additionalProperties": False,
                "properties": {"a": {}},
                "required": ["z"],
            }
        )
    with pytest.raises(SchemaValidationError, match="undeclared property"):
        check_object_schema(
            {
                "type": "object",
                "additionalProperties": False,
                "properties": {"a": {}},
                "required": [7],
            }
        )


def test_check_object_schema_validates_the_top_level_pattern() -> None:
    with pytest.raises(SchemaValidationError, match="requires a maxLength"):
        check_object_schema(
            {
                "type": "object",
                "additionalProperties": False,
                "properties": {"a": {}},
                "pattern": "^a$",
            }
        )
    with pytest.raises(SchemaValidationError, match="pattern must be a string"):
        check_object_schema(
            {
                "type": "object",
                "additionalProperties": False,
                "properties": {"a": {}},
                "pattern": 7,
            }
        )
    with pytest.raises(SchemaValidationError, match="pattern must be a string"):
        check_object_schema(
            {
                "type": "object",
                "additionalProperties": False,
                "properties": {"a": {"pattern": 7}},
            }
        )


def test_check_object_schema_returns_the_schema() -> None:
    schema = {
        "type": "object",
        "additionalProperties": False,
        "properties": {"a": {"type": "string", "maxLength": 4}},
        "required": ["a"],
    }
    assert check_object_schema(schema) is schema


def test_bounded_helpers_reject_bad_values() -> None:
    with pytest.raises(ToolEnvelopeError, match="must be a string"):
        _bounded_identifier(7, "call_id")
    with pytest.raises(ToolEnvelopeError, match="safe identifier"):
        _bounded_identifier("has space", "call_id")
    with pytest.raises(ToolEnvelopeError, match="must be a string"):
        _bounded_version(7, "tool_version")
    with pytest.raises(ToolEnvelopeError, match="safe version string"):
        _bounded_version("has space", "tool_version")
    with pytest.raises(ToolEnvelopeError, match="must be a string"):
        _bounded_text(7, "workspace")
    with pytest.raises(ToolEnvelopeError, match="must be nonempty"):
        _bounded_text("   ", "workspace")
    with pytest.raises(ToolEnvelopeError, match="bounded length"):
        _bounded_text("x" * 20, "workspace", maximum=10)


def test_bounded_helpers_strip_surrounding_space() -> None:
    assert _bounded_identifier("  call-1  ", "call_id") == "call-1"
    assert _bounded_version(" 1.0.0 ", "tool_version") == "1.0.0"
    assert _bounded_text(" /tmp/ws ", "workspace") == "/tmp/ws"


def test_positive_number_rejects_bad_values() -> None:
    with pytest.raises(ToolEnvelopeError, match="must be a number"):
        _positive_number(True, "deadline_seconds")
    with pytest.raises(ToolEnvelopeError, match="must be a number"):
        _positive_number("30", "deadline_seconds")
    with pytest.raises(ToolEnvelopeError, match="finite and positive"):
        _positive_number(0.0, "deadline_seconds")
    with pytest.raises(ToolEnvelopeError, match="finite and positive"):
        _positive_number(float("inf"), "deadline_seconds")
    assert _positive_number(1, "deadline_seconds") == 1.0


def test_positive_integer_rejects_bad_values() -> None:
    with pytest.raises(ToolEnvelopeError, match="must be an integer"):
        _positive_integer(True, "max_output_bytes")
    with pytest.raises(ToolEnvelopeError, match="must be an integer"):
        _positive_integer("7", "max_output_bytes")
    with pytest.raises(ToolEnvelopeError, match="must be positive"):
        _positive_integer(0, "max_output_bytes")
    assert _positive_integer(7, "max_output_bytes") == 7


def test_frozen_object_rejects_bad_shapes() -> None:
    with pytest.raises(ToolEnvelopeError, match="must be a JSON object"):
        _frozen_object(["a"], "arguments")
    with pytest.raises(ToolEnvelopeError, match="keys must be strings"):
        _frozen_object(cast("Mapping[str, Any]", {7: 1}), "arguments")
    frozen = _frozen_object({"a": 1}, "arguments")
    with pytest.raises(TypeError):
        cast("dict[str, Any]", frozen)["b"] = 2


def test_frozen_object_reports_unhashable_values(monkeypatch: pytest.MonkeyPatch) -> None:
    def explode(_value: object) -> object:
        raise TypeError("unhashable")

    monkeypatch.setattr(protocol_module, "MappingProxyType", cast("Any", explode))
    with pytest.raises(ToolEnvelopeError, match="is not a JSON object"):
        _frozen_object({"a": 1}, "arguments")


def test_capability_tuple_sorts_and_deduplicates() -> None:
    assert _capability_tuple(["write", "read", "read"], "granted") == ("read", "write")
    assert _capability_tuple(frozenset({"read"}), "granted") == ("read",)


def test_capability_tuple_rejects_non_collections() -> None:
    with pytest.raises(ToolEnvelopeError, match="collection of capability names"):
        _capability_tuple("read", "granted")
    with pytest.raises(ToolEnvelopeError, match="collection of capability names"):
        _capability_tuple(b"read", "granted")
    with pytest.raises(ToolEnvelopeError, match="collection of capability names"):
        _capability_tuple(cast(Any, 7), "granted")


def test_capability_tuple_validates_entries() -> None:
    with pytest.raises(ToolEnvelopeError, match="entry must be a string"):
        _capability_tuple([7], "granted")
    with pytest.raises(ToolEnvelopeError, match="entry must be a safe identifier"):
        _capability_tuple(["has space"], "granted")


def test_run_status_accepts_enums_and_strings() -> None:
    assert _run_status(ToolRunStatus.SUCCEEDED, "status") is ToolRunStatus.SUCCEEDED
    assert _run_status("succeeded", "status") is ToolRunStatus.SUCCEEDED
    with pytest.raises(ToolEnvelopeError, match="not a tool run status"):
        _run_status("elsewhere", "status")


def test_resolve_workspace_path_rejects_bad_relative_paths(tmp_path: Path) -> None:
    with pytest.raises(ToolWorkspaceError, match="nonempty strings"):
        resolve_workspace_path(tmp_path, "   ")
    with pytest.raises(ToolWorkspaceError, match="nonempty strings"):
        resolve_workspace_path(tmp_path, cast(str, 7))
    with pytest.raises(ToolWorkspaceError, match="null bytes or backslashes"):
        resolve_workspace_path(tmp_path, "a\x00b")
    with pytest.raises(ToolWorkspaceError, match="null bytes or backslashes"):
        resolve_workspace_path(tmp_path, "a\\b")
    with pytest.raises(ToolWorkspaceError, match="bounded length"):
        resolve_workspace_path(tmp_path, "a" * 2_000)


def test_resolve_workspace_path_rejects_absolute_and_parent_paths(tmp_path: Path) -> None:
    with pytest.raises(ToolWorkspaceError, match="must be relative"):
        resolve_workspace_path(tmp_path, "/etc/passwd")
    with pytest.raises(ToolWorkspaceError, match="unusable segment count"):
        resolve_workspace_path(tmp_path, "./")
    with pytest.raises(ToolWorkspaceError, match="unusable segment count"):
        resolve_workspace_path(tmp_path, "/".join(["a"] * (MAX_PATH_SEGMENTS + 1)))
    with pytest.raises(ToolWorkspaceError, match="unsafe segment"):
        resolve_workspace_path(tmp_path, "../escape")
    with pytest.raises(ToolWorkspaceError, match="unsafe segment"):
        resolve_workspace_path(tmp_path, "a/../b")


def test_resolve_workspace_path_requires_an_existing_directory(tmp_path: Path) -> None:
    with pytest.raises(ToolWorkspaceError, match="does not exist"):
        resolve_workspace_path(tmp_path / "absent", "a")


def test_resolve_workspace_path_refuses_symlinked_segments(tmp_path: Path) -> None:
    real = tmp_path / "real"
    real.mkdir()
    (tmp_path / "link").symlink_to(real, target_is_directory=True)
    with pytest.raises(ToolWorkspaceError, match="cannot traverse symlinks"):
        resolve_workspace_path(tmp_path, "link/report.json")
    with pytest.raises(ToolWorkspaceError, match="cannot traverse symlinks"):
        resolve_workspace_path(tmp_path, "./link/./report.json")


def test_resolve_workspace_path_returns_a_path_inside_the_workspace(tmp_path: Path) -> None:
    (tmp_path / "outputs").mkdir()
    assert resolve_workspace_path(tmp_path, "./outputs/./report.json") == (
        tmp_path / "outputs" / "report.json"
    )


def test_request_envelope_rejects_wrong_kind() -> None:
    with pytest.raises(ToolEnvelopeError, match="kind must be a ToolEnvelopeKind"):
        _request(kind="request")
    with pytest.raises(ToolEnvelopeError, match="must be a request envelope"):
        _request(kind=ToolEnvelopeKind.RESPONSE)


def test_request_envelope_rejects_a_relative_workspace() -> None:
    with pytest.raises(ToolEnvelopeError, match="must be an absolute path"):
        _request(workspace="relative/workspace")


def test_request_envelope_rejects_oversized_arguments() -> None:
    with pytest.raises(JsonEnvelopeError, match="bounded length"):
        _request(arguments=cast("dict[str, Any]", {"a": "x" * (MAX_STRING_LENGTH + 1)}))
    with pytest.raises(JsonEnvelopeError, match="bounded size"):
        _request(
            arguments=cast(
                "dict[str, Any]",
                {f"k{index}": "x" * 4_000 for index in range(80)},
            )
        )


def test_request_envelope_rejects_bad_numeric_limits() -> None:
    with pytest.raises(ToolEnvelopeError, match="deadline_seconds"):
        _request(deadline_seconds=0.0)
    with pytest.raises(ToolEnvelopeError, match="max_output_bytes"):
        _request(max_output_bytes=0)
    with pytest.raises(ToolEnvelopeError, match="max_artifact_bytes"):
        _request(max_artifact_bytes=cast(int, True))
    with pytest.raises(ToolEnvelopeError, match="granted_capabilities"):
        _request(granted_capabilities=cast(Any, "read"))


def test_request_envelope_digests_bind_the_exact_payload() -> None:
    request = _request()
    assert request.arguments_json == '{"operation":"beam_volume"}'
    assert request.arguments_digest == sha256_hex(request.arguments_json)
    payload = request.to_payload()
    assert payload["kind"] == "request"
    assert payload["granted_capabilities"] == []
    assert request.envelope_digest == canonical_json_digest(payload)
    assert ToolRequestEnvelope.from_json(request.to_json()) == request


def test_request_envelope_from_payload_rejects_non_mappings() -> None:
    with pytest.raises(ToolEnvelopeError, match="must be a JSON object"):
        ToolRequestEnvelope.from_json("[]")


def test_request_envelope_from_payload_requires_every_field() -> None:
    with pytest.raises(ToolEnvelopeError, match="call_id must be"):
        ToolRequestEnvelope.from_payload({})


def test_request_envelope_from_call_requires_a_tool_call(tmp_path: Path) -> None:
    with pytest.raises(TypeError, match="must be a ToolCall"):
        ToolRequestEnvelope.from_call(
            cast(Any, object()),
            workspace=tmp_path,
            deadline_seconds=1.0,
            max_output_bytes=1,
            max_artifact_bytes=1,
            granted_capabilities=(),
        )


def test_request_envelope_from_call_carries_the_call_identity(tmp_path: Path) -> None:
    from kala_agent.contracts import SideEffectLevel, ToolCall

    call = ToolCall(
        call_id="call-9",
        tool_id="mechanical.reference-simulator",
        tool_version="1.0.0",
        side_effect_level=SideEffectLevel.NONE,
        arguments_json='{"operation":"bar_summary"}',
        timeout_seconds=5.0,
        issued_at=STARTED_AT,
    )
    request = ToolRequestEnvelope.from_call(
        call,
        workspace=tmp_path,
        deadline_seconds=2.0,
        max_output_bytes=10,
        max_artifact_bytes=10,
        granted_capabilities=["artifact.read", "artifact.read"],
    )
    assert request.call_id == "call-9"
    assert request.arguments["operation"] == "bar_summary"
    assert request.workspace == tmp_path.resolve().as_posix()
    assert request.granted_capabilities == ("artifact.read",)


def test_response_envelope_rejects_wrong_kind() -> None:
    with pytest.raises(ToolEnvelopeError, match="must be a response envelope"):
        ToolResponseEnvelope(
            protocol_version=PROTOCOL_VERSION,
            kind=ToolEnvelopeKind.REQUEST,
            call_id="call-1",
            tool_id="tool",
            tool_version="1.0.0",
            status=ToolRunStatus.SUCCEEDED,
            result=cast("dict[str, Any]", {}),
            error_code=None,
            error_message=None,
            artifacts=(),
            duration_ms=0.0,
        )


def test_response_envelope_requires_an_error_code_for_failures() -> None:
    with pytest.raises(ToolEnvelopeError, match="require an error code"):
        ToolResponseEnvelope.failed_envelope(
            call_id="call-1",
            tool_id="tool",
            tool_version="1.0.0",
            error_code=None,
        )
    for status in (ToolRunStatus.DENIED, ToolRunStatus.TIMED_OUT):
        with pytest.raises(ToolEnvelopeError, match="require an error code"):
            ToolResponseEnvelope.failed_envelope(
                call_id="call-1",
                tool_id="tool",
                tool_version="1.0.0",
                status=status,
                error_code=None,
            )


def test_response_envelope_rejects_results_on_failures() -> None:
    with pytest.raises(ToolEnvelopeError, match="cannot carry a result payload"):
        ToolResponseEnvelope(
            protocol_version=PROTOCOL_VERSION,
            kind=ToolEnvelopeKind.RESPONSE,
            call_id="call-1",
            tool_id="tool",
            tool_version="1.0.0",
            status=ToolRunStatus.FAILED,
            result=cast("dict[str, Any]", {"ok": True}),
            error_code="tool_error",
            error_message="failed",
            artifacts=(),
            duration_ms=0.0,
        )


def test_response_envelope_requires_a_result_on_success() -> None:
    with pytest.raises(ToolEnvelopeError, match="require a result payload"):
        ToolResponseEnvelope.succeeded_envelope(
            call_id="call-1",
            tool_id="tool",
            tool_version="1.0.0",
            result=cast("Mapping[str, Any]", {}),
        ) if False else ToolResponseEnvelope(
            protocol_version=PROTOCOL_VERSION,
            kind=ToolEnvelopeKind.RESPONSE,
            call_id="call-1",
            tool_id="tool",
            tool_version="1.0.0",
            status=ToolRunStatus.SUCCEEDED,
            result=None,
            error_code=None,
            error_message=None,
            artifacts=(),
            duration_ms=0.0,
        )


def test_response_envelope_rejects_bad_artifacts() -> None:
    with pytest.raises(ToolEnvelopeError, match="must be ArtifactRef"):
        ToolResponseEnvelope.succeeded_envelope(
            call_id="call-1",
            tool_id="tool",
            tool_version="1.0.0",
            result=cast("dict[str, Any]", {"ok": True}),
            artifacts=cast("Sequence[ArtifactRef]", ["not-a-ref"]),
        )
    with pytest.raises(ToolEnvelopeError, match="must be ArtifactRef"):
        ToolResponseEnvelope.succeeded_envelope(
            call_id="call-1",
            tool_id="tool",
            tool_version="1.0.0",
            result=cast("dict[str, Any]", {"ok": True}),
            artifacts=cast("Sequence[ArtifactRef]", cast(Any, "not-a-tuple")),
        )


def test_response_envelope_rejects_bad_durations() -> None:
    for duration in (True, "1.0"):
        with pytest.raises(ToolEnvelopeError, match="must be a number"):
            _succeeded(duration_ms=duration)
    with pytest.raises(ToolEnvelopeError, match="finite and nonnegative"):
        _succeeded(duration_ms=-1.0)
    with pytest.raises(ToolEnvelopeError, match="finite and nonnegative"):
        _succeeded(duration_ms=float("inf"))


def test_response_envelope_rejects_a_non_object_result() -> None:
    with pytest.raises(ToolEnvelopeError, match="result must be a JSON object"):
        ToolResponseEnvelope(
            protocol_version=PROTOCOL_VERSION,
            kind=ToolEnvelopeKind.RESPONSE,
            call_id="call-1",
            tool_id="tool",
            tool_version="1.0.0",
            status=ToolRunStatus.SUCCEEDED,
            result=cast("Mapping[str, Any]", ["nope"]),
            error_code=None,
            error_message=None,
            artifacts=(),
            duration_ms=0.0,
        )


def test_response_envelope_renders_enum_error_codes() -> None:
    envelope = _failed(error_code=ToolErrorCode.TIMEOUT)
    assert envelope.error_code == "timeout"
    assert envelope.to_payload()["error_code"] == "timeout"
    assert ToolErrorCode(envelope.error_code or "") is ToolErrorCode.TIMEOUT


def test_response_envelope_rejects_unsafe_error_fields() -> None:
    with pytest.raises(ToolEnvelopeError, match="error_code must be a safe identifier"):
        _failed(error_code="has space")
    with pytest.raises(ToolEnvelopeError, match="error_message must be nonempty"):
        _failed(error_message="   ")
    with pytest.raises(ToolEnvelopeError, match="error_message exceeds the bounded length"):
        _failed(error_message="x" * 3_000)


def test_response_envelope_reports_status_and_digests() -> None:
    envelope = _succeeded()
    assert envelope.succeeded is True
    assert envelope.result_json == '{"ok":true}'
    assert envelope.result_digest == sha256_hex('{"ok":true}')
    assert ToolResponseEnvelope.from_json(envelope.to_json()) == envelope
    assert envelope.to_payload()["status"] == "succeeded"
    assert envelope.to_payload()["artifacts"] == []


def test_response_envelope_digest_uses_an_empty_object_for_failures() -> None:
    envelope = _failed()
    assert envelope.succeeded is False
    assert envelope.result_json is None
    assert envelope.result_digest == sha256_hex("{}")
    assert envelope.to_payload()["result"] is None


def test_response_envelope_carries_artifacts(tmp_path: Path) -> None:
    artifact = _artifact()
    envelope = _succeeded(artifacts=(artifact,))
    assert envelope.artifacts == (artifact,)
    assert envelope.to_payload()["artifacts"] == [artifact.model_dump(mode="json")]


def test_response_envelope_from_payload_rejects_malformed_artifacts() -> None:
    with pytest.raises(ToolEnvelopeError, match="artifacts must be an array"):
        ToolResponseEnvelope.from_payload(cast("JsonObject", {"artifacts": "nope"}))
    with pytest.raises(ToolEnvelopeError, match="must be ArtifactRef objects"):
        ToolResponseEnvelope.from_payload(cast("JsonObject", {"artifacts": ["nope"]}))
    with pytest.raises(ToolEnvelopeError, match="artifact reference is invalid"):
        ToolResponseEnvelope.from_payload(cast("JsonObject", {"artifacts": [{"artifact_id": "a"}]}))


def test_response_envelope_from_payload_requires_every_field() -> None:
    with pytest.raises(ToolEnvelopeError, match="status is not a tool run status"):
        ToolResponseEnvelope.from_payload({})


def test_response_envelope_from_payload_round_trips_artifacts() -> None:
    envelope = _succeeded(artifacts=(_artifact(),), duration_ms=1.5)
    payload = cast("JsonObject", dict(envelope.to_payload()))
    assert ToolResponseEnvelope.from_payload(payload) == envelope


def test_response_envelope_from_json_requires_an_object() -> None:
    with pytest.raises(ToolEnvelopeError, match="must be a JSON object"):
        ToolResponseEnvelope.from_json("[]")


def test_response_envelope_to_result_requires_aware_timestamps() -> None:
    envelope = _succeeded()
    with pytest.raises(ToolEnvelopeError, match="started_at must include a timezone"):
        envelope.to_result(started_at=datetime(2026, 1, 1), finished_at=FINISHED_AT)
    with pytest.raises(ToolEnvelopeError, match="finished_at must include a timezone"):
        envelope.to_result(started_at=STARTED_AT, finished_at=datetime(2026, 1, 1))
    with pytest.raises(ToolEnvelopeError, match="cannot precede"):
        envelope.to_result(started_at=FINISHED_AT, finished_at=STARTED_AT)


def test_response_envelope_to_result_records_the_call() -> None:
    envelope = _succeeded(artifacts=(_artifact(),), duration_ms=2.0)
    result = envelope.to_result(started_at=STARTED_AT, finished_at=FINISHED_AT)
    assert result.call_id == "call-1"
    assert result.status is ToolRunStatus.SUCCEEDED
    assert result.artifacts == (_artifact(),)
    assert result.duration_ms == 2.0
    failure = _failed().to_result(started_at=STARTED_AT, finished_at=FINISHED_AT)
    assert failure.error_code == "tool_error"
    assert failure.result_json is None


def test_subprocess_handler_validates_its_descriptor() -> None:
    from kala_agent.tools.simulator import simulator_descriptor

    descriptor = simulator_descriptor()
    with pytest.raises(ToolEnvelopeError, match="descriptor must be a ToolDescriptor"):
        SubprocessToolHandler(
            descriptor=cast(Any, object()),
            entry_point="kala_agent.tools.simulator:invoke",
        )
    assert (
        SubprocessToolHandler(
            descriptor=descriptor,
            entry_point="kala_agent.tools.simulator:invoke",
        ).descriptor
        is descriptor
    )


def test_subprocess_handler_validates_the_entry_point() -> None:
    from kala_agent.tools.simulator import simulator_descriptor

    descriptor = simulator_descriptor()
    with pytest.raises(ToolEnvelopeError, match="entry_point must be a string"):
        SubprocessToolHandler(descriptor=descriptor, entry_point=cast(Any, 7))
    with pytest.raises(ToolEnvelopeError, match="module:attribute"):
        SubprocessToolHandler(descriptor=descriptor, entry_point="no-separator")
    with pytest.raises(ToolEnvelopeError, match="module:attribute"):
        SubprocessToolHandler(descriptor=descriptor, entry_point=":invoke")
    with pytest.raises(ToolEnvelopeError, match="module:attribute"):
        SubprocessToolHandler(descriptor=descriptor, entry_point="module:")
    with pytest.raises(ToolEnvelopeError, match="must be identifiers"):
        SubprocessToolHandler(descriptor=descriptor, entry_point="1module:invoke")
    with pytest.raises(ToolEnvelopeError, match="must be identifiers"):
        SubprocessToolHandler(descriptor=descriptor, entry_point="module:1invoke")


def test_subprocess_handler_validates_interpreter_arguments() -> None:
    from kala_agent.tools.simulator import simulator_descriptor

    descriptor = simulator_descriptor()
    for arguments in (cast(Any, "-B"), cast(Any, cast("list[str]", ["-B", 7]))):
        with pytest.raises(ToolEnvelopeError, match="sequence of strings"):
            SubprocessToolHandler(
                descriptor=descriptor,
                entry_point="module:invoke",
                interpreter_args=arguments,
            )
    handler = SubprocessToolHandler(
        descriptor=descriptor,
        entry_point="module:invoke",
        interpreter_args=cast("tuple[str, ...]", ["-B", "-s"]),
    )
    assert handler.interpreter_args == ("-B", "-s")


def test_subprocess_handler_refuses_bootstrap_replacing_flags() -> None:
    from kala_agent.tools.simulator import simulator_descriptor

    descriptor = simulator_descriptor()
    for flag in ("-c", "-E", "-I", "-m", "not-a-flag"):
        with pytest.raises(ToolEnvelopeError, match="cannot replace the worker bootstrap"):
            SubprocessToolHandler(
                descriptor=descriptor,
                entry_point="module:invoke",
                interpreter_args=(flag,),
            )


def test_subprocess_handler_cannot_run_in_process() -> None:
    from kala_agent.tools.simulator import simulator_handler

    with pytest.raises(protocol_module.ToolProtocolError, match="dispatched by the executor"):
        simulator_handler().invoke(_request())


def test_build_worker_command_requires_a_handler() -> None:
    with pytest.raises(TypeError, match="SubprocessToolHandler"):
        build_worker_command(cast(Any, object()))


def test_build_worker_command_reports_a_missing_interpreter(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    from kala_agent.tools.simulator import simulator_handler

    monkeypatch.setattr(sys, "executable", "")
    with pytest.raises(protocol_module.ToolProtocolError, match="no python interpreter"):
        build_worker_command(simulator_handler())


def test_build_worker_command_prefers_the_supplied_interpreter() -> None:
    from kala_agent.tools.simulator import simulator_handler

    command = build_worker_command(simulator_handler(), python_executable="/usr/bin/python3")
    assert command[0] == "/usr/bin/python3"
    assert command[-1] == "kala_agent.tools.simulator:invoke"
    assert "-c" in command
    assert protocol_module.WORKER_BOOTSTRAP in command


def test_build_child_environment_pins_every_protected_key(tmp_path: Path) -> None:
    environment = build_child_environment(
        workspace=tmp_path,
        import_roots=(Path("/opt/roots"),),
        extra={"EXTRA_FLAG": "1"},
    )
    assert environment["HOME"] == tmp_path.resolve().as_posix()
    assert environment["TMPDIR"] == environment["HOME"]
    assert environment["PYTHONPATH"] == "/opt/roots"
    assert environment["KALA_TOOL_PROTOCOL"] == PROTOCOL_VERSION
    assert environment["EXTRA_FLAG"] == "1"
    assert environment["PATH"] == "/usr/local/bin:/usr/bin:/bin"


def test_build_child_environment_defaults_the_import_path(tmp_path: Path) -> None:
    environment = build_child_environment(workspace=tmp_path)
    assert environment["PYTHONPATH"] == ""
    assert "PYTHONSAFEPATH" in environment
    assert "PYTHONNOUSERSITE" in environment


def test_build_child_environment_rejects_protected_overrides(tmp_path: Path) -> None:
    for key in ("HOME", "PATH", "PYTHONPATH", "TMPDIR"):
        with pytest.raises(ToolEnvelopeError, match="protected by the executor"):
            build_child_environment(workspace=tmp_path, extra={key: "/elsewhere"})


def test_build_child_environment_rejects_malformed_overrides(tmp_path: Path) -> None:
    with pytest.raises(ToolEnvelopeError, match="simple names"):
        build_child_environment(workspace=tmp_path, extra={"has space": "1"})
    with pytest.raises(ToolEnvelopeError, match="simple names"):
        build_child_environment(workspace=tmp_path, extra=cast("Mapping[str, str]", {7: "1"}))
    with pytest.raises(ToolEnvelopeError, match="null-free strings"):
        build_child_environment(workspace=tmp_path, extra={"EXTRA": cast("str", 7)})
    with pytest.raises(ToolEnvelopeError, match="null-free strings"):
        build_child_environment(workspace=tmp_path, extra={"EXTRA": "a\x00b"})


def test_read_request_stream_enforces_the_size_bound() -> None:
    with pytest.raises(ToolEnvelopeError, match="bounded size"):
        read_request_stream(cast("Any", _FakeStream(b"x" * (MAX_JSON_BYTES + 1))))


def test_read_request_stream_decodes_a_valid_envelope() -> None:
    request = _request()
    decoded = read_request_stream(cast("Any", _FakeStream(request.to_json().encode())))
    assert decoded == request


def test_write_response_stream_handles_text_and_binary_streams() -> None:
    envelope = _succeeded()
    text = _FakeTextStream()
    write_response_stream(envelope, cast("Any", text))
    assert text.written.endswith("\n")
    binary = _FakeTextStream()
    write_response_stream(envelope, cast("Any", binary))
    assert binary.written == envelope.to_json() + "\n"


def test_summarize_error_reports_only_the_type_when_empty() -> None:
    assert summarize_error(ValueError("   ")) == "ValueError"
    assert summarize_error(ValueError("boom")) == "ValueError: boom"
    assert summarize_error(ValueError("boom boom")) == "ValueError: boom boom"
    assert summarize_error(ValueError("at /tmp/ws/x"), replacement="/tmp/ws") == (
        "ValueError: at <workspace>/x"
    )


def test_sanitize_child_stderr_keeps_first_and_last_lines() -> None:
    payload = b"Traceback (most recent call last):\n  File x\nValueError: boom"
    assert sanitize_child_stderr(payload) == (
        "Traceback (most recent call last): | ... | ValueError: boom"
    )


def test_sanitize_child_stderr_keeps_two_lines_verbatim() -> None:
    assert sanitize_child_stderr(b"first\nsecond") == "first | second"


def test_sanitize_child_stderr_collapses_long_output() -> None:
    payload = b"first\nsecond\nthird\nfourth\nfifth"
    assert sanitize_child_stderr(payload) == "first | ... | fifth"


def test_sanitize_child_stderr_handles_empty_and_control_input() -> None:
    assert sanitize_child_stderr(b"") == ""
    assert sanitize_child_stderr(b"\x00\x01only") == "only"
    assert sanitize_child_stderr(b"a\r\nb") == "a | b"
    assert sanitize_child_stderr(b"  \n \n ") == ""


def test_sanitize_child_stderr_redacts_the_workspace() -> None:
    assert sanitize_child_stderr(b"failed in /tmp/ws-1234", replacement="/tmp/ws-1234") == (
        "failed in <workspace>"
    )
    assert sanitize_child_stderr(b"failed in /tmp/ws", replacement="") == "failed in /tmp/ws"


def test_sanitize_child_stderr_bounds_the_sample() -> None:
    payload = ("x" * 4_000).encode()
    assert len(sanitize_child_stderr(payload)) <= 512


class _FakeStream:
    def __init__(self, data: bytes) -> None:
        self._data = data
        self.flushed = False

    def read(self, size: int) -> bytes:
        return self._data[:size]


class _FakeTextStream(TextIOBase):
    def __init__(self) -> None:
        super().__init__()
        self.written = ""

    def write(self, text: str) -> int:
        self.written += text
        return len(text)
