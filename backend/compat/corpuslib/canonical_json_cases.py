"""Canonical JSON oracle cases.

Every expected value is produced by calling the live
``kala_agent.tools.protocol`` functions. Inputs are stored as JSON text or as
a tagged typed source so that an integer, a float, a string, and a Decimal
never collapse into the same fixture representation.
"""

from __future__ import annotations

import json
from typing import Final

from corpuslib.common import sha256_text

MAX_STORED_TEXT: Final[int] = 4096


def _materialize(source: dict[str, object]) -> object:
    kind = source["kind"]
    if kind == "json_text":
        return json.loads(str(source["text"]))
    if kind == "utf8_text":
        return str(source["text"])
    if kind == "repeat_string":
        return str(source["char"]) * int(source["count"])  # type: ignore[arg-type]
    if kind == "nested_arrays":
        value: object = 0
        for _ in range(int(source["levels"])):  # type: ignore[arg-type]
            value = [value]
        return value
    if kind == "array_of_strings":
        item = str(source["item"])
        return [item for _ in range(int(source["count"]))]  # type: ignore[arg-type]
    if kind == "array_of_repeated_strings":
        item = str(source["char"]) * int(source["item_count"])  # type: ignore[arg-type]
        return [item for _ in range(int(source["array_count"]))]  # type: ignore[arg-type]
    if kind == "array_of_ints":
        return [0 for _ in range(int(source["count"]))]  # type: ignore[arg-type]
    if kind == "object_key_length":
        return {str(source["char"]) * int(source["count"]): 1}  # type: ignore[arg-type]
    if kind == "object_nonstring_key":
        return {1: "not a string key"}
    if kind == "decimal_literal":
        from decimal import Decimal

        return Decimal(str(source["text"]))
    if kind == "float_literal":
        return float(str(source["literal"]))
    raise ValueError(f"unknown canonical JSON source kind: {kind!r}")


def _dumps_expected(
    source: dict[str, object],
    *,
    max_bytes: int | None = None,
) -> dict[str, object]:
    from kala_agent.tools.protocol import canonical_json_dumps

    try:
        value = _materialize(source)
        if max_bytes is None:
            output = canonical_json_dumps(value)
        else:
            output = canonical_json_dumps(value, max_bytes=max_bytes)
    except Exception as error:
        return {"ok": False, "error_type": type(error).__name__, "message": str(error)}
    expected: dict[str, object] = {
        "ok": True,
        "output_length": len(output),
        "output_sha256": sha256_text(output),
        "digest": sha256_text(output),
    }
    if len(output) <= MAX_STORED_TEXT:
        expected["output"] = output
    return expected


def _loads_expected(source: dict[str, object]) -> dict[str, object]:
    from kala_agent.tools.protocol import canonical_json_dumps, strict_json_loads

    kind = source["kind"]
    if kind == "hex":
        payload: str | bytes = bytes.fromhex(str(source["hex"]))
    elif kind == "quoted_repeat":
        payload = '"' + str(source["char"]) * int(source["count"]) + '"'  # type: ignore[arg-type]
    else:
        payload = str(source["text"])
    try:
        value = strict_json_loads(payload)
    except Exception as error:
        return {"ok": False, "error_type": type(error).__name__, "message": str(error)}
    canonical = canonical_json_dumps(value)
    expected: dict[str, object] = {
        "ok": True,
        "value_type": type(value).__name__,
        "canonical_length": len(canonical),
        "canonical_sha256": sha256_text(canonical),
        "digest": sha256_text(canonical),
    }
    if len(canonical) <= MAX_STORED_TEXT:
        expected["canonical"] = canonical
    return expected


def _dumps_case(
    case_id: str,
    source: dict[str, object],
    *,
    max_bytes: int | None = None,
    note: str | None = None,
) -> dict[str, object]:
    case: dict[str, object] = {
        "id": case_id,
        "operation": "dumps",
        "input": source,
        "expected": _dumps_expected(source, max_bytes=max_bytes),
    }
    if max_bytes is not None:
        case["max_bytes"] = max_bytes
    if note:
        case["note"] = note
    return case


def _loads_case(
    case_id: str,
    source: dict[str, object],
    *,
    note: str | None = None,
) -> dict[str, object]:
    case: dict[str, object] = {
        "id": case_id,
        "operation": "loads",
        "input": source,
        "expected": _loads_expected(source),
    }
    if note:
        case["note"] = note
    return case


def build() -> dict[str, object]:
    cases: list[dict[str, object]] = [
        _dumps_case("float.negative_zero", {"kind": "json_text", "text": "-0.0"}),
        _dumps_case("float.positive_zero", {"kind": "json_text", "text": "0.0"}),
        _dumps_case("float.integral_one", {"kind": "json_text", "text": "1.0"}),
        _dumps_case("integer.one", {"kind": "json_text", "text": "1"}),
        _dumps_case(
            "float.lower_exponent_threshold",
            {"kind": "json_text", "text": "1e-05"},
            note="CPython repr switches to exponent form below 1e-4.",
        ),
        _dumps_case("float.below_lower_threshold", {"kind": "json_text", "text": "1e-04"}),
        _dumps_case("float.upper_exponent_threshold", {"kind": "json_text", "text": "1e16"}),
        _dumps_case("float.below_upper_threshold", {"kind": "json_text", "text": "1e15"}),
        _dumps_case("float.fractional", {"kind": "json_text", "text": "1.5"}),
        _dumps_case("float.subnormal_min", {"kind": "json_text", "text": "5e-324"}),
        _dumps_case(
            "float.underflow_to_zero",
            {"kind": "json_text", "text": "1e-400"},
            note="A finite decimal literal that rounds to 0.0 is accepted.",
        ),
        _dumps_case("float.negative_lower_exponent", {"kind": "json_text", "text": "-1e-05"}),
        _dumps_case(
            "float.max_finite",
            {"kind": "json_text", "text": "1.7976931348623157e308"},
        ),
        _dumps_case("int.two_pow_70", {"kind": "json_text", "text": str(2**70)}),
        _dumps_case("int.negative_two_pow_70", {"kind": "json_text", "text": str(-(2**70))}),
        _dumps_case("int.ten_pow_100", {"kind": "json_text", "text": str(10**100)}),
        _dumps_case(
            "string.ascii_controls",
            {"kind": "json_text", "text": '"\\u0000\\u001f\\u007f"'},
        ),
        _dumps_case(
            "string.ascii_escapes",
            {"kind": "json_text", "text": '"quote \\" slash \\\\ tab \\t newline \\n"'},
        ),
        _dumps_case("string.greek_escaped", {"kind": "json_text", "text": '"\\u03ba"'},),
        _dumps_case(
            "string.supplementary_escaped",
            {"kind": "json_text", "text": '"\\ud83d\\ude00"'},
        ),
        _dumps_case(
            "string.raw_unicode",
            {"kind": "json_text", "text": '"κ 😀"'},
            note="Parsed as UTF-8 input; emitted as ASCII escapes.",
        ),
        _dumps_case(
            "string.lone_high_surrogate_escaped",
            {"kind": "json_text", "text": '"\\ud800"'},
        ),
        _dumps_case(
            "string.lone_low_surrogate_escaped",
            {"kind": "json_text", "text": '"\\udfff"'},
        ),
        _dumps_case(
            "object.key_ordering",
            {"kind": "json_text", "text": '{"b":1,"a":2,"Z":3,"z":4}'},
        ),
        _dumps_case(
            "object.unicode_key_ordering",
            {"kind": "json_text", "text": '{"z":1,"ä":2,"a":3}'},
        ),
        _dumps_case("literal.boolean", {"kind": "json_text", "text": "true"}),
        _dumps_case("literal.null", {"kind": "json_text", "text": "null"}),
        _dumps_case("string.numeric_string", {"kind": "json_text", "text": '"1.0"'}),
        _dumps_case(
            "string.solidus_not_escaped",
            {"kind": "json_text", "text": '"a/b"'},
            note="CPython does not escape a solidus.",
        ),
        _dumps_case(
            "object.nonstring_key",
            {"kind": "object_nonstring_key"},
            note="Only string keys are JSON-serializable.",
        ),
        _dumps_case(
            "decimal.not_json_serializable",
            {"kind": "decimal_literal", "text": "1.00"},
            note="Budget costs are converted to str; raw Decimal is not a JSON value.",
        ),
        _dumps_case(
            "object.mixed",
            {
                "kind": "json_text",
                "text": (
                    '{"int":1,"float":1.0,"negative_zero":-0.0,"big":'
                    + str(2**70)
                    + ',"text":"κ","bool":false,"null":null,"array":[3,2,1]}'
                ),
            },
        ),
        _dumps_case(
            "string.length_at_limit",
            {"kind": "repeat_string", "char": "a", "count": 32_768},
        ),
        _dumps_case(
            "string.length_over_limit",
            {"kind": "repeat_string", "char": "a", "count": 32_769},
        ),
        _dumps_case(
            "object.key_at_limit",
            {"kind": "object_key_length", "char": "k", "count": 256},
        ),
        _dumps_case(
            "object.key_over_limit",
            {"kind": "object_key_length", "char": "k", "count": 257},
        ),
        _dumps_case(
            "array.items_at_limit",
            {"kind": "array_of_ints", "count": 2_048},
        ),
        _dumps_case(
            "array.items_over_limit",
            {"kind": "array_of_ints", "count": 2_049},
        ),
        _dumps_case("depth.at_limit", {"kind": "nested_arrays", "levels": 48}),
        _dumps_case("depth.over_limit", {"kind": "nested_arrays", "levels": 49}),
        _dumps_case(
            "bytes.at_limit",
            {
                "kind": "array_of_repeated_strings",
                "char": "a",
                "item_count": 32_767,
                "array_count": 31,
            },
            note="1,015,871 canonical bytes: below the 1,048,576-byte limit.",
        ),
        _dumps_case(
            "bytes.over_limit",
            {
                "kind": "array_of_repeated_strings",
                "char": "a",
                "item_count": 32_767,
                "array_count": 32,
            },
            note="1,048,673 canonical bytes: above the 1,048,576-byte limit.",
        ),
        _dumps_case(
            "bytes.custom_max_bytes_over_limit",
            {"kind": "json_text", "text": '{"a":1}'},
            max_bytes=6,
            note="The caller-supplied byte ceiling is honoured; the 7-byte result is refused.",
        ),
        _dumps_case("nonfinite.nan", {"kind": "float_literal", "literal": "nan"}),
        _dumps_case("nonfinite.infinity", {"kind": "float_literal", "literal": "inf"}),
        _dumps_case(
            "nonfinite.negative_infinity",
            {"kind": "float_literal", "literal": "-inf"},
        ),
        _loads_case("loads.object_duplicate_keys", {"kind": "text", "text": '{"a":1,"a":2}'}),
        _loads_case("loads.object_duplicate_keys_same_value", {"kind": "text", "text": '{"a":1,"a":1}'}),
        _loads_case("loads.constant_nan", {"kind": "text", "text": "NaN"}),
        _loads_case("loads.constant_infinity", {"kind": "text", "text": "Infinity"}),
        _loads_case("loads.constant_negative_infinity", {"kind": "text", "text": "-Infinity"}),
        _loads_case("loads.float_overflow", {"kind": "text", "text": "1e999"}),
        _loads_case("loads.not_json", {"kind": "text", "text": "{"}),
        _loads_case("loads.empty", {"kind": "text", "text": ""}),
        _loads_case("loads.trailing_data", {"kind": "text", "text": "{} {}"}),
        _loads_case("loads.utf8_bom", {"kind": "text", "text": "\ufeff{}"}),
        _loads_case("loads.invalid_utf8", {"kind": "hex", "hex": "ff"}),
        _loads_case("loads.top_level_scalar", {"kind": "text", "text": "42"}),
        _loads_case("loads.lone_surrogate", {"kind": "text", "text": '"\\ud800"'}),
        _loads_case("loads.bigint", {"kind": "text", "text": str(2**70)}),
        _loads_case("loads.float_exponent", {"kind": "text", "text": "1e-05"}),
        _loads_case(
            "loads.key_ordering_is_not_preserved",
            {"kind": "text", "text": '{"b":1,"a":2}'},
        ),
        _loads_case("loads.depth_at_limit", {"kind": "text", "text": "[" * 48 + "0" + "]" * 48}),
        _loads_case("loads.depth_over_limit", {"kind": "text", "text": "[" * 49 + "0" + "]" * 49}),
        _loads_case(
            "loads.string_over_length_limit",
            {"kind": "text", "text": '"' + "a" * 32_769 + '"'},
        ),
        _loads_case(
            "loads.payload_over_byte_limit",
            {"kind": "quoted_repeat", "char": "a", "count": 1_048_576 + 1},
        ),
    ]
    return {
        "corpus_schema_version": 1,
        "kind": "canonical_json",
        "oracle_module": "kala_agent.tools.protocol",
        "functions": ("canonical_json_dumps", "strict_json_loads", "canonical_json_digest"),
        "limits": {
            "MAX_JSON_BYTES": 1_048_576,
            "MAX_STRING_LENGTH": 32_768,
            "MAX_COLLECTION_ITEMS": 2_048,
            "MAX_KEY_LENGTH": 256,
            "MAX_JSON_DEPTH": 48,
        },
        "encoding_rules": {
            "ensure_ascii": True,
            "separators": [",", ":"],
            "sort_keys": True,
            "allow_nan": False,
            "duplicate_keys": "rejected",
            "non_finite": "rejected",
            "float_formatter": "CPython json.dumps/repr shortest round-trip",
        },
        "cases": cases,
    }


__all__ = ("build",)
