"""Live Python oracle for Unicode numeric input parity.

Reads a JSON request on stdin and writes a JSON response on stdout. Supported
request keys:

- ``float_cases``: list of strings; each is passed to ``float()`` and the
  result is reported as ``repr(value)`` (exact text, no NaN/Infinity JSON
  tokens) or as the exception class name.
- ``budget_cases``: list of objects validated with the real
  ``kala_agent.contracts.RunBudget`` model; accepted models are reported via
  ``model_dump(mode="json")``.
- ``unicode_digits``: when true, reports every Unicode ``Nd`` codepoint with its
  decimal value, every numeric-but-not-decimal codepoint, every
  ``str.isspace()`` codepoint and the ``unicodedata`` version.

This is test-only tooling; the Rust crate never invokes it at runtime.
"""

import json
import os
import sys
import unicodedata

sys.path.insert(0, os.environ.get("KALA_SRC", ""))

from kala_agent.contracts import RunBudget  # noqa: E402


def float_rows(cases):
    rows = []
    for text in cases:
        try:
            rows.append({"ok": True, "text": repr(float(text))})
        except Exception as error:  # noqa: BLE001 - oracle reports the class name
            rows.append({"ok": False, "error": type(error).__name__})
    return rows


def budget_rows(cases):
    rows = []
    for case in cases:
        try:
            budget = RunBudget(**case)
        except Exception:  # noqa: BLE001 - acceptance parity only, not text
            rows.append({"ok": False})
        else:
            rows.append({"ok": True, "value": budget.model_dump(mode="json")})
    return rows


def unicode_report():
    digits = []
    non_decimal_numeric = []
    spaces = []
    for codepoint in range(sys.maxunicode + 1):
        character = chr(codepoint)
        value = unicodedata.decimal(character, None)
        if value is not None:
            digits.append([codepoint, value])
        elif character.isnumeric():
            non_decimal_numeric.append(codepoint)
        if character.isspace():
            spaces.append(codepoint)
    return {
        "digits": digits,
        "non_decimal_numeric": non_decimal_numeric,
        "spaces": spaces,
        "unidata_version": unicodedata.unidata_version,
    }


def main():
    payload = json.load(sys.stdin)
    result = {}
    if "float_cases" in payload:
        result["floats"] = float_rows(payload["float_cases"])
    if "budget_cases" in payload:
        result["budgets"] = budget_rows(payload["budget_cases"])
    if payload.get("unicode_digits"):
        result.update(unicode_report())
    json.dump(result, sys.stdout)


if __name__ == "__main__":
    main()
