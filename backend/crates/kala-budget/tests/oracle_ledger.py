"""Live Python oracle for the ledger differential test.

Reads a JSON command list on stdin and executes it against the real
`kala_agent.kernel.budget` implementation, writing a JSON result list on
stdout. The Rust test executes the same command list with the native ledger
and compares the results. Test-only tooling.
"""

import json
import os
import sys
from decimal import Decimal

sys.path.insert(0, os.environ.get("KALA_SRC", ""))

from kala_agent.contracts import RunBudget  # noqa: E402
from kala_agent.kernel import budget as kernel  # noqa: E402


def decimal_cost(value):
    if isinstance(value, str):
        return Decimal(value)
    return value


def kwargs_of(command):
    values = dict(command.get("kwargs", {}))
    if "cost" in values:
        values["cost"] = decimal_cost(values["cost"])
    return values


def call_kwargs(command):
    values = {
        key: value for key, value in command.items() if key not in {"cmd"}
    }
    if "cost" in values:
        values["cost"] = decimal_cost(values["cost"])
    return values


def budget_of(payload):
    values = dict(payload)
    if "max_cost" in values:
        values["max_cost"] = decimal_cost(values["max_cost"])
    return RunBudget(**values)


def find_reservation(ledger, reservation_id):
    for reservation in ledger.reservations():
        if reservation.reservation_id == reservation_id:
            return reservation
    raise kernel.BudgetReservationUnknownError(f"unknown reservation: {reservation_id}")


def execute(ledger, command):
    name = command["cmd"]
    if name == "new":
        created = kernel.BudgetLedger(budget_of(command["budget"]))
        return created, None
    if name == "record_model_call":
        usage = ledger.record_model_call(**call_kwargs(command))
        return ledger, usage.as_dict()
    if name == "record_tool_call":
        usage = ledger.record_tool_call(**call_kwargs(command))
        return ledger, usage.as_dict()
    if name == "consume":
        usage = ledger.consume(**kwargs_of(command))
        return ledger, usage.as_dict()
    if name == "reserve":
        reservation = ledger.reserve(**kwargs_of(command))
        return ledger, {
            "reservation": reservation.as_dict(),
            "usage": ledger.usage.as_dict(),
            "state": ledger.export_state(),
        }
    if name == "commit":
        reservation = find_reservation(ledger, command["id"])
        usage = ledger.commit(reservation, **kwargs_of(command))
        return ledger, usage.as_dict()
    if name == "commit_plain":
        reservation = find_reservation(ledger, command["id"])
        usage = ledger.commit(reservation)
        return ledger, usage.as_dict()
    if name == "commit_wrong":
        reservation = kernel.BudgetReservation(
            reservation_id=command["id"],
            usage=kernel.BudgetUsage(**{
                key: decimal_cost(value) if key == "cost" else value
                for key, value in command["usage"].items()
            }),
        )
        usage = ledger.commit(reservation)
        return ledger, usage.as_dict()
    if name == "release":
        reserved = ledger.release(find_reservation(ledger, command["id"]))
        return ledger, reserved.as_dict()
    if name == "export":
        return ledger, ledger.export_state()
    if name == "import":
        ledger.import_state(command["state"])
        return ledger, ledger.export_state()
    if name == "available":
        return ledger, ledger.available.as_dict()
    if name == "usage":
        return ledger, ledger.usage.as_dict()
    if name == "reserved":
        return ledger, ledger.reserved.as_dict()
    if name == "reservations":
        return ledger, [reservation.as_dict() for reservation in ledger.reservations()]
    if name == "is_exhausted":
        dimension = command.get("dimension")
        return ledger, ledger.is_exhausted(dimension)
    if name == "can_consume":
        return ledger, ledger.can_consume(**kwargs_of(command))
    raise ValueError(f"unknown command {name}")


def main():
    commands = json.load(sys.stdin)
    ledger = None
    results = []
    for command in commands:
        try:
            ledger, value = execute(ledger, command)
            results.append({"ok": True, "value": value})
        except Exception as error:  # noqa: BLE001 - oracle reports the class name
            results.append(
                {
                    "ok": False,
                    "error_type": type(error).__name__,
                    "message": str(error),
                }
            )
    json.dump(results, sys.stdout)


if __name__ == "__main__":
    main()
