"""Live Python oracle for the decimal differential test (stdlib only).

Reads a JSON request on stdin and writes a JSON response on stdout. This is
test-only tooling; the Rust crate never invokes it at runtime.
"""

import json
import sys
from decimal import Decimal


def describe(value):
    sign, digits, exponent = value.as_tuple()
    return {
        "ok": True,
        "str": str(value),
        "tuple": [sign, list(digits), exponent],
    }


def describe_parse(text):
    try:
        return describe(Decimal(text))
    except Exception as error:  # noqa: BLE001 - oracle reports the class name
        return {"ok": False, "error": type(error).__name__}


def describe_operation(left, right, apply):
    try:
        return describe(apply(left, right))
    except Exception as error:  # noqa: BLE001
        return {"ok": False, "error": type(error).__name__}


def main():
    payload = json.load(sys.stdin)
    decimal_rows = []
    for case in payload["decimal_cases"]:
        left = describe_parse(case["left"])
        right = describe_parse(case["right"])
        row = {
            "left": left,
            "right": right,
            "add": describe_operation(
                Decimal(case["left"]), Decimal(case["right"]), lambda a, b: a + b
            )
            if left["ok"] and right["ok"]
            else {"ok": False, "error": "PreviousError"},
            "sub": describe_operation(
                Decimal(case["left"]), Decimal(case["right"]), lambda a, b: a - b
            )
            if left["ok"] and right["ok"]
            else {"ok": False, "error": "PreviousError"},
        }
        decimal_rows.append(row)

    duration_rows = []
    for case in payload["duration_cases"]:
        seconds = case["seconds"]
        try:
            micros = round(seconds * 1_000_000)
            duration_rows.append({"ok": True, "seconds": micros / 1_000_000})
        except Exception as error:  # noqa: BLE001
            duration_rows.append({"ok": False, "error": type(error).__name__})

    duration_sum_rows = []
    for pair in payload["duration_sums"]:
        try:
            micros = []
            for seconds in pair:
                stored = round(seconds * 1_000_000) / 1_000_000
                micros.append(round(stored * 1_000_000))
            combined = (micros[0] + micros[1]) / 1_000_000
            regridded = round(combined * 1_000_000) / 1_000_000
            duration_sum_rows.append({"ok": True, "seconds": regridded})
        except Exception as error:  # noqa: BLE001
            duration_sum_rows.append({"ok": False, "error": type(error).__name__})

    json.dump(
        {
            "decimal": decimal_rows,
            "duration": duration_rows,
            "duration_sums": duration_sum_rows,
        },
        sys.stdout,
    )


if __name__ == "__main__":
    main()
