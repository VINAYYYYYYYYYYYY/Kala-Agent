from __future__ import annotations

import re
import time
from collections.abc import Mapping
from typing import Final

import pytest

from kala_agent.contracts import SideEffectLevel, ToolDescriptor
from kala_agent.tools import (
    MAX_PATTERN_INPUT_LENGTH,
    MAX_PATTERN_LENGTH,
    SchemaValidationError,
    SubprocessToolHandler,
    ToolRegistrationError,
    ToolRegistry,
    canonical_json_digest,
    check_object_schema,
    validate_against_schema,
)

ATTACK_INPUT: Final[str] = "a" * 20 + "b"

AMBIGUOUS_REPETITION: Final[tuple[str, ...]] = (
    r"(a|aa)+$",
    r"(a|a)+$",
    r"(a|ab)*$",
    r"(|a)*",
    r"^(?:foo|bar)+$",
    r"(?:a|ab){1,64}",
    r"(a*)(a*)$",
    r"a*a*a*a*a*b",
    r"a*a*a*a*a*a*b$",
    r"[a-z]*[a-z]*[a-z]*q",
    r"\w+\s?\w+",
    r"^\w+@\w+\.\w+$",
    r"[-+]?[0-9]{1,3}",
    r"(a+)+$",
    r"(a*)+$",
    r"(?:a{1,64})*",
    r"^(a|a?)+$",
    r"(x+x+)+y",
    r"(.+)*z",
    r"(\w+\s?)*$",
    r"^(\s+)+x",
    r"(a+)*(b+)*",
)

UNSUPPORTED_CONSTRUCTS: Final[tuple[str, ...]] = (
    r"(a)\1",
    r"(?P<n>a)(?P=n)",
    r"(?=x)y",
    r"(?<=x)y",
    r"(?!x)y",
    r"(?<!x)y",
    r"(a)?(?(1)b|c)",
    r"(?i)meter",
    r"(?im-sx:meter)",
    r"(?>a+)b",
    r"a*+",
    r"a++",
    r".*",
    r".+",
    r".{2,}",
    r"^.*$",
)

OVER_BUDGET_REPETITION: Final[tuple[str, ...]] = (
    r"a{0,16}b{0,16}c{0,16}d{0,16}",
    r"a{0,32}b{0,32}c{0,32}d{0,32}",
    r"(?:ab){0,64}(?:cd){0,64}",
    r"a{0,2048}",
    r"a{0,1001}",
    r"a{100000}",
    r"a{0,}" * 8,
    "a" * (MAX_PATTERN_LENGTH + 1),
)

ADMITTED_SUBSET: Final[tuple[tuple[str, str], ...]] = (
    (r"^bar-[0-9]{2}$", "bar-42"),
    (r"^[A-Za-z0-9._-]{1,64}$", "report.v1_final"),
    (r"^/[\w./-]{0,200}$", "/a/b-c.json"),
    (r"^\d{4}-\d{2}-\d{2}$", "2026-05-06"),
    (r"^(?:abc)+$", "abcabcabc"),
    (r"^(ab)+$", "abab"),
    (r"^(?:ab){2,3}$", "ababab"),
    (r"^(a+)$", "aaa"),
    (r"^[a-z]+$", "abc"),
    (r"^a?b$", "b"),
    (r"^[0-9]{1,3}$", "102"),
    (r"^\(\)$", "()"),
    (r"a|bc", "bc"),
    (r"^[^,]{0,10}$", "no commas"),
)


def _schema(pattern: str, **overrides: object) -> Mapping[str, object]:
    schema: dict[str, object] = {
        "type": "string",
        "maxLength": MAX_PATTERN_INPUT_LENGTH,
        "pattern": pattern,
    }
    schema.update(overrides)
    return schema


def _attack_input(limit: int = MAX_PATTERN_INPUT_LENGTH) -> str:
    return (ATTACK_INPUT * limit)[:limit]


def test_repetition_that_can_be_split_two_ways_is_refused() -> None:
    for pattern in AMBIGUOUS_REPETITION:
        with pytest.raises(SchemaValidationError):
            validate_against_schema(_attack_input(), _schema(pattern))


def test_constructs_outside_the_admitted_subset_are_refused() -> None:
    for pattern in UNSUPPORTED_CONSTRUCTS:
        with pytest.raises(SchemaValidationError):
            validate_against_schema(_attack_input(), _schema(pattern))


def test_repetition_beyond_the_backtracking_budget_is_refused() -> None:
    for pattern in OVER_BUDGET_REPETITION:
        with pytest.raises(SchemaValidationError):
            validate_against_schema(_attack_input(), _schema(pattern))


def test_registration_refuses_a_schema_whose_pattern_is_ambiguous() -> None:
    for pattern in ("^(a|aa)+$", "a*a*a*a*a*b", r"^(\w+\s?)*$", "^(a+)+$"):
        with pytest.raises(SchemaValidationError):
            check_object_schema(
                {
                    "additionalProperties": False,
                    "properties": {
                        "value": {"maxLength": 32, "pattern": pattern, "type": "string"}
                    },
                    "required": ["value"],
                    "type": "object",
                }
            )


def test_a_refused_pattern_is_never_compiled(monkeypatch: pytest.MonkeyPatch) -> None:
    compiled: list[str] = []
    real_compile = re.compile

    def recording(pattern: str, flags: int = 0) -> re.Pattern[str]:
        compiled.append(pattern)
        return real_compile(pattern, flags)

    monkeypatch.setattr(re, "compile", recording)
    for pattern in (*AMBIGUOUS_REPETITION, *UNSUPPORTED_CONSTRUCTS, *OVER_BUDGET_REPETITION):
        with pytest.raises(SchemaValidationError):
            validate_against_schema(_attack_input(64), _schema(pattern, maxLength=64))
    assert compiled == []
    validate_against_schema("bar-42", _schema(r"^bar-[0-9]{2}$", maxLength=64))
    assert compiled == [r"^bar-[0-9]{2}$"]


def test_the_admitted_subset_still_matches_and_still_binds() -> None:
    unmatched = "!" * MAX_PATTERN_LENGTH
    for pattern, sample in ADMITTED_SUBSET:
        schema = _schema(pattern, maxLength=MAX_PATTERN_INPUT_LENGTH)
        validate_against_schema(sample, schema)
        with pytest.raises(SchemaValidationError):
            validate_against_schema(unmatched, schema)


def test_refusing_a_hostile_pattern_is_bounded_work() -> None:
    started = time.monotonic()
    for pattern in AMBIGUOUS_REPETITION:
        with pytest.raises(SchemaValidationError):
            validate_against_schema(_attack_input(), _schema(pattern))
    for pattern, sample in ADMITTED_SUBSET:
        validate_against_schema(sample, _schema(pattern))
    assert time.monotonic() - started < 5.0


def test_a_pattern_needs_a_bounded_max_length() -> None:
    for schema in (
        {"type": "string", "pattern": "^a+$"},
        {"type": "string", "maxLength": 0, "pattern": "^a+$"},
        {"type": "string", "maxLength": MAX_PATTERN_INPUT_LENGTH + 1, "pattern": "^a+$"},
        {"type": "string", "maxLength": 4_096, "pattern": "^a+$"},
        {"type": "string", "maxLength": 32, "pattern": 7},
    ):
        with pytest.raises(SchemaValidationError):
            validate_against_schema("aaa", schema)


def test_a_registry_refuses_a_tool_whose_schema_pattern_is_ambiguous() -> None:
    registry = ToolRegistry()
    schema: Mapping[str, object] = {
        "additionalProperties": False,
        "properties": {"value": {"maxLength": 32, "pattern": "^(a|aa)+$", "type": "string"}},
        "required": ["value"],
        "type": "object",
    }
    output_schema: Mapping[str, object] = {
        "additionalProperties": False,
        "properties": {"value": {"type": "string"}},
        "required": ["value"],
        "type": "object",
    }
    descriptor = ToolDescriptor(
        tool_id="test.redos",
        version="1.0.0",
        name="ReDoS",
        description="Declares a pattern that can be split two ways",
        input_schema_digest=canonical_json_digest(schema),
        output_schema_digest=canonical_json_digest(output_schema),
        side_effect_level=SideEffectLevel.READ,
        required_capabilities=("design.read",),
        timeout_seconds=5.0,
    )
    handler = SubprocessToolHandler(
        descriptor=descriptor,
        entry_point="kala_agent.tools.simulator:invoke",
    )
    with pytest.raises(ToolRegistrationError, match="alternatives"):
        registry.register(
            descriptor,
            handler,
            input_schema=schema,
            output_schema=output_schema,
        )
    assert registry.tool_ids() == ()
