"""Decimal and budget oracle cases.

The source accepts arbitrary finite nonnegative ``Decimal`` values. These
cases freeze stored scale, very large and very small exponents, values wider
than the 28-digit default context, context-rounded sums, the microsecond
duration grid, and ties-to-even rounding. They are deliberately hostile to a
fixed 96-bit/28-scale representation.
"""

from __future__ import annotations

from decimal import Decimal
from typing import Final

from corpuslib.common import call_or_error


def _decimal_tuple(value: Decimal) -> dict[str, object]:
    sign, digits, exponent = value.as_tuple()
    return {
        "sign": sign,
        "digits": list(digits),
        "exponent": exponent,
        "str": str(value),
        "is_finite": value.is_finite(),
    }


def _decode_cases(values: list[str]) -> list[dict[str, object]]:
    cases: list[dict[str, object]] = []
    for index, text in enumerate(values):
        cases.append(
            {
                "id": f"decode.{index:02d}",
                "input": text,
                "expected": _decimal_tuple(Decimal(text)),
            }
        )
    return cases


DECIMAL_STRINGS: Final[tuple[str, ...]] = (
    "0",
    "0.0",
    "0E-1000000",
    "1",
    "1.00",
    "0.10",
    "0.2",
    "1E+3",
    "100.000",
    "1.2300",
    "0.0000000000000000000000000000001",
    "123456789012345678901234567890.12345",
    "1.23456789012345678901234567890123456789",
    "1e-30",
    "1e+40",
    "1e-999999",
    "1e+999999",
)


def _run_budget_cases() -> list[dict[str, object]]:
    from kala_agent.contracts import RunBudget

    cases: list[dict[str, object]] = []
    for index, text in enumerate(DECIMAL_STRINGS):
        result = call_or_error(
            lambda text=text: RunBudget(max_cost=Decimal(text)).model_dump(mode="json")
        )
        cases.append({"id": f"run_budget.{index:02d}", "max_cost": text, "expected": result})
    for index, text in enumerate(("-1", "-0.0001", "NaN", "sNaN", "Infinity", "-Infinity")):
        result = call_or_error(
            lambda text=text: RunBudget(max_cost=Decimal(text)).model_dump(mode="json")
        )
        cases.append({"id": f"run_budget.reject.{index:02d}", "max_cost": text, "expected": result})
    return cases


def _usage_cases() -> list[dict[str, object]]:
    from kala_agent.kernel.budget import BudgetUsage

    cases: list[dict[str, object]] = []

    def usage(cost: str) -> dict[str, object]:
        return BudgetUsage(cost=Decimal(cost)).as_dict()

    for index, text in enumerate(
        ("0", "0.0", "1.00", "0.10", "0.2", "1E+3", "1e-30", "1e+40", "1e-999999", "1e+999999")
    ):
        cases.append(
            {"id": f"usage.single.{index:02d}", "cost": text, "expected": usage(text)}
        )

    sums = (
        ("0.10", "0.2"),
        ("0.1", "0.2"),
        ("1.00", "2.00"),
        ("1e+40", "1e-30"),
        ("1e+999999", "1e-999999"),
        ("1.23456789012345678901234567890123456789", "1e-40"),
        ("123456789012345678901234567890.12345", "0.00001"),
    )
    for index, (left, right) in enumerate(sums):
        expected = call_or_error(
            lambda left=left, right=right: (
                BudgetUsage(cost=Decimal(left)) + BudgetUsage(cost=Decimal(right))
            ).as_dict()
        )
        cases.append(
            {
                "id": f"usage.sum.{index:02d}",
                "left": left,
                "right": right,
                "expected": expected,
                "note": "Decimal addition uses the live context; scale is not normalized",
            }
        )

    for index, (left, right) in enumerate(
        (("0.5", "0.5"), ("1", "2"), ("0.0000005", "0.0000015"))
    ):
        expected = call_or_error(
            lambda left=left, right=right: (
                BudgetUsage(cost=Decimal(left)) + BudgetUsage(cost=Decimal(right))
            ).as_dict()
        )
        cases.append(
            {
                "id": f"usage.subtract_or_identity.{index:02d}",
                "left": left,
                "right": right,
                "expected": expected,
            }
        )
    return cases


def _duration_cases() -> list[dict[str, object]]:
    from kala_agent.kernel.budget import BudgetUsage

    values: tuple[float, ...] = (
        0.0,
        1e-07,
        5e-07,
        1.5e-06,
        2.5e-06,
        0.0000015,
        1.2345675,
        1.2345685,
        123.4567895,
        3600.0,
    )
    cases: list[dict[str, object]] = []
    for index, value in enumerate(values):
        expected = call_or_error(lambda value=value: BudgetUsage(duration_seconds=value).as_dict())
        cases.append(
            {
                "id": f"duration.{index:02d}",
                "seconds": value,
                "expected": expected,
                "note": "Durations are held on a whole-microsecond grid using round()",
            }
        )
    sums = ((0.0000005, 0.0000005), (0.1, 0.2), (1.234567, 2.345679))
    for index, pair in enumerate(sums):
        expected = call_or_error(
            lambda pair=pair: (
                BudgetUsage(duration_seconds=pair[0])
                + BudgetUsage(duration_seconds=pair[1])
            ).as_dict()
        )
        cases.append({"id": f"duration.sum.{index:02d}", "seconds": list(pair), "expected": expected})
    return cases


def _ledger_cases() -> list[dict[str, object]]:
    from kala_agent.contracts import RunBudget
    from kala_agent.kernel.budget import BudgetLedger

    def scenario() -> dict[str, object]:
        ledger = BudgetLedger(
            RunBudget(
                max_model_calls=4,
                max_tool_calls=5,
                max_total_tokens=1_000,
                max_artifact_bytes=10_000,
                max_duration_seconds=10.0,
                max_cost=Decimal("1.00"),
                max_replanning_attempts=2,
            )
        )
        observations: list[dict[str, object]] = []
        observations.append({"step": "initial", "state": ledger.export_state()})
        observations.append(
            {"step": "model_call", "usage": ledger.record_model_call(tokens=100, cost="0.10").as_dict()}
        )
        observations.append(
            {
                "step": "tool_call",
                "usage": ledger.record_tool_call(
                    duration_seconds=0.0000005, artifact_bytes=64, cost="0.20"
                ).as_dict(),
            }
        )
        reservation = ledger.reserve(total_tokens=200, cost=Decimal("0.25"))
        observations.append(
            {
                "step": "reserve",
                "reservation": reservation.as_dict(),
                "state": ledger.export_state(),
            }
        )
        observations.append(
            {
                "step": "commit_reservation",
                "usage": ledger.commit(
                    reservation, total_tokens=150, cost=Decimal("0.10")
                ).as_dict(),
            }
        )
        observations.append({"step": "final", "state": ledger.export_state()})

        imported = BudgetLedger()
        imported.import_state(ledger.export_state())
        observations.append(
            {
                "step": "import_round_trip",
                "state": imported.export_state(),
                "equal_to_export": imported.export_state() == ledger.export_state(),
            }
        )
        exhaustion = call_or_error(lambda: ledger.record_cost(Decimal("1.0")).as_dict())
        observations.append({"step": "exhaust_cost", "expected": exhaustion})
        return {"observations": observations}

    return [{"id": "ledger.scenario", "expected": call_or_error(scenario)}]


def _clamp_cases() -> list[dict[str, object]]:
    from kala_agent.domains import clamp_unit_interval

    values: list[dict[str, object]] = [
        {"value": 0.0},
        {"value": 1.0},
        {"value": -1.0},
        {"value": 2.0},
        {"value": 0.1234564},
        {"value": 0.1234565},
        {"value": 0.1234575},
        {"value": 0.9999995},
        {"value": 1e-7},
        {"float_literal": "inf"},
        {"float_literal": "nan"},
    ]
    cases: list[dict[str, object]] = []
    for index, item in enumerate(values):
        value = float(item["float_literal"]) if "float_literal" in item else item["value"]
        cases.append(
            {
                "id": f"clamp.{index:02d}",
                "input": item,
                "expected": call_or_error(lambda value=value: clamp_unit_interval(value)),
            }
        )
    return cases


def build() -> dict[str, object]:
    return {
        "corpus_schema_version": 1,
        "kind": "decimal_budget",
        "oracle_modules": (
            "kala_agent.kernel.budget",
            "kala_agent.contracts.models.RunBudget",
            "kala_agent.domains.clamp_unit_interval",
        ),
        "facts": {
            "source_range": (
                "arbitrary finite nonnegative Decimal; no fixed scale or magnitude bound"
            ),
            "python_decimal_context_precision": 28,
            "python_decimal_rounding": "ROUND_HALF_EVEN",
            "cost_state_representation": "str(Decimal) preserves stored scale and exponent",
            "duration_grid": "whole microseconds via round(seconds * 1_000_000)",
            "duration_rounding": "ties to even because round() has no ndigits argument",
            "rust_decimal_fixed_96_bit_is_insufficient": True,
        },
        "decimal_strings": list(DECIMAL_STRINGS),
        "decode_cases": _decode_cases(list(DECIMAL_STRINGS)),
        "run_budget_cases": _run_budget_cases(),
        "usage_cases": _usage_cases(),
        "duration_cases": _duration_cases(),
        "ledger_cases": _ledger_cases(),
        "clamp_cases": _clamp_cases(),
    }


__all__ = ("DECIMAL_STRINGS", "build")
