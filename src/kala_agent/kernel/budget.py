from __future__ import annotations

import math
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
from enum import StrEnum
from threading import RLock
from typing import Final, TypeAlias, cast

from kala_agent.contracts import RunBudget

Numeric: TypeAlias = int | float | Decimal

BUDGET_STATE_VERSION: Final[int] = 1

_USAGE_STATE_KEYS: Final[frozenset[str]] = frozenset(
    {
        "model_calls",
        "tool_calls",
        "total_tokens",
        "artifact_bytes",
        "duration_seconds",
        "cost",
        "replanning_attempts",
    }
)

_RESERVATION_STATE_KEYS: Final[frozenset[str]] = frozenset({"reservation_id", "usage"})

_BUDGET_STATE_KEYS: Final[frozenset[str]] = frozenset(
    {
        "version",
        "budget",
        "usage",
        "reserved",
        "reservations",
        "next_reservation",
    }
)

_RUN_BUDGET_KEYS: Final[frozenset[str]] = frozenset(
    {
        "max_model_calls",
        "max_tool_calls",
        "max_total_tokens",
        "max_artifact_bytes",
        "max_duration_seconds",
        "max_cost",
        "max_replanning_attempts",
    }
)


def _nonnegative_int(value: object, field_name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise TypeError(f"{field_name} must be an integer")
    if value < 0:
        raise ValueError(f"{field_name} must be nonnegative")
    return value


def _nonnegative_decimal(value: object, field_name: str) -> Decimal:
    if isinstance(value, bool):
        raise TypeError(f"{field_name} must be numeric")
    try:
        parsed = value if isinstance(value, Decimal) else Decimal(str(value))
    except (InvalidOperation, TypeError, ValueError) as error:
        raise ValueError(f"{field_name} must be a finite nonnegative decimal") from error
    if not parsed.is_finite() or parsed < 0:
        raise ValueError(f"{field_name} must be a finite nonnegative decimal")
    return parsed


def _nonnegative_duration(value: object, field_name: str) -> float:
    if isinstance(value, bool):
        raise TypeError(f"{field_name} must be numeric")
    if not isinstance(value, (int, float, Decimal, str)):
        raise TypeError(f"{field_name} must be numeric")
    try:
        parsed = float(value)
    except (TypeError, ValueError, OverflowError) as error:
        raise ValueError(f"{field_name} must be finite and nonnegative") from error
    if not math.isfinite(parsed) or parsed < 0.0:
        raise ValueError(f"{field_name} must be finite and nonnegative")
    return _seconds(_microseconds(parsed))


# Durations are held on a whole-microsecond grid and summed as integers, so a
# reservation released in any order leaves the ledger exactly where it was.
_MICROSECONDS_PER_SECOND: Final[int] = 1_000_000


def _microseconds(seconds: float) -> int:
    return round(seconds * _MICROSECONDS_PER_SECOND)


def _seconds(microseconds: int) -> float:
    return microseconds / _MICROSECONDS_PER_SECOND


class BudgetError(RuntimeError):
    pass


class BudgetStateError(BudgetError, ValueError):
    pass


class BudgetCounterError(BudgetError, ValueError):
    pass


class BudgetReservationUnknownError(BudgetError, LookupError):
    pass


class BudgetReservationMismatchError(BudgetError, ValueError):
    pass


class BudgetDimension(StrEnum):
    MODEL_CALLS = "model_calls"
    TOOL_CALLS = "tool_calls"
    TOTAL_TOKENS = "total_tokens"
    ARTIFACT_BYTES = "artifact_bytes"
    DURATION_SECONDS = "duration_seconds"
    COST = "cost"
    REPLANNING_ATTEMPTS = "replanning_attempts"


@dataclass(frozen=True, slots=True)
class BudgetUsage:
    model_calls: int = 0
    tool_calls: int = 0
    total_tokens: int = 0
    artifact_bytes: int = 0
    duration_seconds: float = 0.0
    cost: Decimal = Decimal("0")
    replanning_attempts: int = 0

    def __post_init__(self) -> None:
        object.__setattr__(self, "model_calls", _nonnegative_int(self.model_calls, "model_calls"))
        object.__setattr__(self, "tool_calls", _nonnegative_int(self.tool_calls, "tool_calls"))
        object.__setattr__(
            self,
            "total_tokens",
            _nonnegative_int(self.total_tokens, "total_tokens"),
        )
        object.__setattr__(
            self,
            "artifact_bytes",
            _nonnegative_int(self.artifact_bytes, "artifact_bytes"),
        )
        object.__setattr__(
            self,
            "duration_seconds",
            _nonnegative_duration(self.duration_seconds, "duration_seconds"),
        )
        object.__setattr__(self, "cost", _nonnegative_decimal(self.cost, "cost"))
        object.__setattr__(
            self,
            "replanning_attempts",
            _nonnegative_int(self.replanning_attempts, "replanning_attempts"),
        )

    @property
    def tokens(self) -> int:
        return self.total_tokens

    @property
    def replanning(self) -> int:
        return self.replanning_attempts

    def __add__(self, other: BudgetUsage) -> BudgetUsage:
        if not isinstance(other, BudgetUsage):
            raise TypeError("other must be a BudgetUsage")
        return BudgetUsage(
            model_calls=self.model_calls + other.model_calls,
            tool_calls=self.tool_calls + other.tool_calls,
            total_tokens=self.total_tokens + other.total_tokens,
            artifact_bytes=self.artifact_bytes + other.artifact_bytes,
            duration_seconds=_seconds(
                _microseconds(self.duration_seconds) + _microseconds(other.duration_seconds)
            ),
            cost=self.cost + other.cost,
            replanning_attempts=self.replanning_attempts + other.replanning_attempts,
        )

    def __sub__(self, other: BudgetUsage) -> BudgetUsage:
        if not isinstance(other, BudgetUsage):
            raise TypeError("other must be a BudgetUsage")
        if self.exceeds(other):
            raise BudgetCounterError("budget counters cannot become negative")
        return BudgetUsage(
            model_calls=self.model_calls - other.model_calls,
            tool_calls=self.tool_calls - other.tool_calls,
            total_tokens=self.total_tokens - other.total_tokens,
            artifact_bytes=self.artifact_bytes - other.artifact_bytes,
            duration_seconds=_seconds(
                _microseconds(self.duration_seconds) - _microseconds(other.duration_seconds)
            ),
            cost=self.cost - other.cost,
            replanning_attempts=self.replanning_attempts - other.replanning_attempts,
        )

    def exceeds(self, other: BudgetUsage) -> bool:
        if not isinstance(other, BudgetUsage):
            raise TypeError("other must be a BudgetUsage")
        return (
            self.model_calls < other.model_calls
            or self.tool_calls < other.tool_calls
            or self.total_tokens < other.total_tokens
            or self.artifact_bytes < other.artifact_bytes
            or self.duration_seconds < other.duration_seconds
            or self.cost < other.cost
            or self.replanning_attempts < other.replanning_attempts
        )

    def as_dict(self) -> dict[str, object]:
        return {
            "model_calls": self.model_calls,
            "tool_calls": self.tool_calls,
            "total_tokens": self.total_tokens,
            "artifact_bytes": self.artifact_bytes,
            "duration_seconds": self.duration_seconds,
            "cost": str(self.cost),
            "replanning_attempts": self.replanning_attempts,
        }


@dataclass(frozen=True, slots=True)
class BudgetSnapshot:
    budget: RunBudget
    usage: BudgetUsage
    reserved: BudgetUsage

    @property
    def remaining(self) -> BudgetUsage:
        return BudgetUsage(
            model_calls=max(
                0,
                self.budget.max_model_calls - self.usage.model_calls - self.reserved.model_calls,
            ),
            tool_calls=max(
                0,
                self.budget.max_tool_calls - self.usage.tool_calls - self.reserved.tool_calls,
            ),
            total_tokens=max(
                0,
                self.budget.max_total_tokens - self.usage.total_tokens - self.reserved.total_tokens,
            ),
            artifact_bytes=max(
                0,
                self.budget.max_artifact_bytes
                - self.usage.artifact_bytes
                - self.reserved.artifact_bytes,
            ),
            duration_seconds=max(
                0.0,
                self.budget.max_duration_seconds
                - self.usage.duration_seconds
                - self.reserved.duration_seconds,
            ),
            cost=max(
                Decimal("0"),
                self.budget.max_cost - self.usage.cost - self.reserved.cost,
            ),
            replanning_attempts=max(
                0,
                self.budget.max_replanning_attempts
                - self.usage.replanning_attempts
                - self.reserved.replanning_attempts,
            ),
        )


class BudgetExhaustedError(BudgetError):
    def __init__(
        self,
        dimension: BudgetDimension,
        requested: Numeric,
        remaining: Numeric,
        limit: Numeric,
    ) -> None:
        self.dimension = dimension
        self.requested = requested
        self.remaining = remaining
        self.limit = limit
        super().__init__(
            f"budget exhausted for {dimension.value}: requested {requested}, "
            f"remaining {remaining}, limit {limit}"
        )


class ModelCallBudgetExhaustedError(BudgetExhaustedError):
    pass


class ToolCallBudgetExhaustedError(BudgetExhaustedError):
    pass


class TokenBudgetExhaustedError(BudgetExhaustedError):
    pass


class ArtifactBudgetExhaustedError(BudgetExhaustedError):
    pass


class DurationBudgetExhaustedError(BudgetExhaustedError):
    pass


class ReplanningBudgetExhaustedError(BudgetExhaustedError):
    pass


class CostBudgetExhaustedError(BudgetExhaustedError):
    pass


BudgetExceeded = BudgetExhaustedError
BudgetExceededError = BudgetExhaustedError
BudgetLimitExceededError = BudgetExhaustedError


@dataclass(frozen=True, slots=True)
class BudgetReservation:
    reservation_id: str
    usage: BudgetUsage

    def as_dict(self) -> dict[str, object]:
        return {"reservation_id": self.reservation_id, "usage": self.usage.as_dict()}


_EXCEPTION_BY_DIMENSION: dict[BudgetDimension, type[BudgetExhaustedError]] = {
    BudgetDimension.MODEL_CALLS: ModelCallBudgetExhaustedError,
    BudgetDimension.TOOL_CALLS: ToolCallBudgetExhaustedError,
    BudgetDimension.TOTAL_TOKENS: TokenBudgetExhaustedError,
    BudgetDimension.ARTIFACT_BYTES: ArtifactBudgetExhaustedError,
    BudgetDimension.DURATION_SECONDS: DurationBudgetExhaustedError,
    BudgetDimension.COST: CostBudgetExhaustedError,
    BudgetDimension.REPLANNING_ATTEMPTS: ReplanningBudgetExhaustedError,
}


def _state_error(message: str) -> BudgetStateError:
    return BudgetStateError(f"budget state is invalid: {message}")


def _state_mapping(value: object, field_name: str) -> Mapping[str, object]:
    if not isinstance(value, Mapping):
        raise _state_error(f"{field_name} must be a mapping")
    for key in value:
        if not isinstance(key, str):
            raise _state_error(f"{field_name} keys must be strings")
    return value


def _state_sequence(value: object, field_name: str) -> Sequence[object]:
    if isinstance(value, (str, bytes, bytearray)) or not isinstance(value, Sequence):
        raise _state_error(f"{field_name} must be a sequence")
    return value


def _state_int(value: object, field_name: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise _state_error(f"{field_name} must be an integer")
    return value


def _state_number(value: object, field_name: str) -> Numeric:
    if isinstance(value, bool) or not isinstance(value, (int, float, str)):
        raise _state_error(f"{field_name} must be numeric")
    return cast(Numeric, value)


def _require_keys(payload: Mapping[str, object], allowed: frozenset[str], field_name: str) -> None:
    unexpected = sorted(set(payload) - allowed)
    if unexpected:
        raise _state_error(f"{field_name} has unexpected keys: {', '.join(unexpected)}")


def _usage_from_state(value: object, field_name: str) -> BudgetUsage:
    payload = _state_mapping(value, field_name)
    _require_keys(payload, _USAGE_STATE_KEYS, field_name)
    try:
        return BudgetUsage(
            model_calls=_nonnegative_int(
                _state_int(payload.get("model_calls", 0), f"{field_name}.model_calls"),
                "model_calls",
            ),
            tool_calls=_nonnegative_int(
                _state_int(payload.get("tool_calls", 0), f"{field_name}.tool_calls"),
                "tool_calls",
            ),
            total_tokens=_nonnegative_int(
                _state_int(payload.get("total_tokens", 0), f"{field_name}.total_tokens"),
                "total_tokens",
            ),
            artifact_bytes=_nonnegative_int(
                _state_int(payload.get("artifact_bytes", 0), f"{field_name}.artifact_bytes"),
                "artifact_bytes",
            ),
            duration_seconds=_nonnegative_duration(
                _state_number(
                    payload.get("duration_seconds", 0.0), f"{field_name}.duration_seconds"
                ),
                "duration_seconds",
            ),
            cost=_nonnegative_decimal(
                _state_number(payload.get("cost", "0"), f"{field_name}.cost"),
                "cost",
            ),
            replanning_attempts=_nonnegative_int(
                _state_int(
                    payload.get("replanning_attempts", 0),
                    f"{field_name}.replanning_attempts",
                ),
                "replanning_attempts",
            ),
        )
    except (TypeError, ValueError) as error:
        raise _state_error(f"{field_name} is invalid: {error}") from error


def _reservation_from_state(value: object) -> tuple[str, BudgetUsage]:
    payload = _state_mapping(value, "reservation")
    _require_keys(payload, _RESERVATION_STATE_KEYS, "reservation")
    reservation_id = payload.get("reservation_id")
    if (
        not isinstance(reservation_id, str)
        or not reservation_id.strip()
        or len(reservation_id) > 256
    ):
        raise _state_error("reservation.reservation_id must be a nonempty bounded string")
    usage = _usage_from_state(payload.get("usage", {}), "reservation.usage")
    return reservation_id.strip(), usage


class BudgetLedger:
    def __init__(self, budget: RunBudget | None = None) -> None:
        self._budget = budget if budget is not None else RunBudget()
        if not isinstance(self._budget, RunBudget):
            raise TypeError("budget must be a RunBudget")
        self._usage = BudgetUsage()
        self._reserved = BudgetUsage()
        self._reservations: dict[str, BudgetUsage] = {}
        self._next_reservation = 1
        self._lock = RLock()

    @property
    def budget(self) -> RunBudget:
        return self._budget

    @property
    def usage(self) -> BudgetUsage:
        with self._lock:
            return self._usage

    @property
    def reserved(self) -> BudgetUsage:
        with self._lock:
            return self._reserved

    @property
    def available(self) -> BudgetUsage:
        with self._lock:
            return self._snapshot_locked().remaining

    @property
    def remaining(self) -> BudgetUsage:
        return self.available

    def snapshot(self) -> BudgetSnapshot:
        with self._lock:
            return self._snapshot_locked()

    def can_consume(self, usage: BudgetUsage | None = None, **kwargs: Numeric) -> bool:
        try:
            charge = self._merge_usage(usage, kwargs)
            with self._lock:
                self._check_locked(charge)
        except (TypeError, ValueError, BudgetExhaustedError):
            return False
        return True

    def check(self, usage: BudgetUsage | None = None, **kwargs: Numeric) -> None:
        charge = self._merge_usage(usage, kwargs)
        with self._lock:
            self._check_locked(charge)

    ensure = check

    def consume(self, usage: BudgetUsage | None = None, **kwargs: Numeric) -> BudgetUsage:
        charge = self._merge_usage(usage, kwargs)
        with self._lock:
            self._check_locked(charge)
            self._usage = self._usage + charge
            return self._usage

    charge = consume
    record = consume

    def record_model_call(
        self,
        count: int = 1,
        *,
        tokens: int = 0,
        duration_seconds: float = 0.0,
        cost: Numeric = Decimal("0"),
    ) -> BudgetUsage:
        return self.consume(
            model_calls=count,
            total_tokens=tokens,
            duration_seconds=duration_seconds,
            cost=cost,
        )

    def record_tool_call(
        self,
        count: int = 1,
        *,
        duration_seconds: float = 0.0,
        artifact_bytes: int = 0,
        cost: Numeric = Decimal("0"),
    ) -> BudgetUsage:
        return self.consume(
            tool_calls=count,
            duration_seconds=duration_seconds,
            artifact_bytes=artifact_bytes,
            cost=cost,
        )

    record_model_calls = record_model_call
    record_tool_calls = record_tool_call

    def record_tokens(self, tokens: int) -> BudgetUsage:
        return self.consume(total_tokens=tokens)

    def record_artifact_bytes(self, artifact_bytes: int) -> BudgetUsage:
        return self.consume(artifact_bytes=artifact_bytes)

    def record_duration(self, duration_seconds: float) -> BudgetUsage:
        return self.consume(duration_seconds=duration_seconds)

    def record_cost(self, cost: Numeric) -> BudgetUsage:
        return self.consume(cost=cost)

    def record_replanning(self, count: int = 1) -> BudgetUsage:
        return self.consume(replanning_attempts=count)

    record_replanning_attempt = record_replanning

    def reserve(self, usage: BudgetUsage | None = None, **kwargs: Numeric) -> BudgetReservation:
        charge = self._merge_usage(usage, kwargs)
        with self._lock:
            self._check_locked(charge)
            reservation_id = self._next_reservation_id()
            self._reservations[reservation_id] = charge
            self._reserved = self._reserved + charge
            return BudgetReservation(reservation_id=reservation_id, usage=charge)

    def commit(
        self,
        reservation: BudgetReservation,
        actual: BudgetUsage | None = None,
        **kwargs: Numeric,
    ) -> BudgetUsage:
        if not isinstance(reservation, BudgetReservation):
            raise TypeError("reservation must be a BudgetReservation")
        charge = (
            self._merge_usage(actual, kwargs)
            if (actual is not None or kwargs)
            else reservation.usage
        )
        with self._lock:
            self._require_reservation_locked(reservation)
            self._check_locked(charge, released=reservation.usage)
            del self._reservations[reservation.reservation_id]
            self._reserved = self._reserved - reservation.usage
            self._usage = self._usage + charge
            return self._usage

    def release(self, reservation: BudgetReservation) -> BudgetUsage:
        if not isinstance(reservation, BudgetReservation):
            raise TypeError("reservation must be a BudgetReservation")
        with self._lock:
            self._require_reservation_locked(reservation)
            reserved_usage = self._reservations[reservation.reservation_id]
            remaining_reserved = self._reserved - reserved_usage
            del self._reservations[reservation.reservation_id]
            self._reserved = remaining_reserved
            return self._reserved

    def reservations(self) -> tuple[BudgetReservation, ...]:
        with self._lock:
            return tuple(
                BudgetReservation(reservation_id=key, usage=value)
                for key, value in self._reservations.items()
            )

    def export_state(self) -> dict[str, object]:
        with self._lock:
            return {
                "version": BUDGET_STATE_VERSION,
                "budget": self._budget.model_dump(mode="json"),
                "usage": self._usage.as_dict(),
                "reserved": self._reserved.as_dict(),
                "reservations": [
                    {"reservation_id": key, "usage": value.as_dict()}
                    for key, value in self._reservations.items()
                ],
                "next_reservation": self._next_reservation,
            }

    def import_state(self, state: Mapping[str, object]) -> None:
        payload = _state_mapping(state, "state")
        _require_keys(payload, _BUDGET_STATE_KEYS, "state")
        version = _state_int(payload.get("version"), "state.version")
        if version != BUDGET_STATE_VERSION:
            raise _state_error(f"unsupported version {version}")
        budget_payload = _state_mapping(payload.get("budget", {}), "state.budget")
        _require_keys(budget_payload, _RUN_BUDGET_KEYS, "state.budget")
        try:
            budget = RunBudget.model_validate(dict(budget_payload))
        except (TypeError, ValueError) as error:
            raise _state_error(f"state.budget is invalid: {error}") from error
        usage = _usage_from_state(payload.get("usage", {}), "state.usage")
        reserved = _usage_from_state(payload.get("reserved", {}), "state.reserved")
        reservations: dict[str, BudgetUsage] = {}
        for item in _state_sequence(payload.get("reservations", ()), "state.reservations"):
            reservation_id, reservation_usage = _reservation_from_state(item)
            if reservation_id in reservations:
                raise _state_error(f"duplicate reservation {reservation_id}")
            reservations[reservation_id] = reservation_usage
        expected = BudgetUsage()
        for reservation_usage in reservations.values():
            expected = expected + reservation_usage
        if expected != reserved:
            raise _state_error("state.reserved does not match the reservation total")
        next_reservation = _state_int(
            payload.get("next_reservation", len(reservations) + 1),
            "state.next_reservation",
        )
        if next_reservation < 1:
            raise _state_error("state.next_reservation must be positive")
        with self._lock:
            self._budget = budget
            self._usage = usage
            self._reserved = reserved
            self._reservations = reservations
            self._next_reservation = max(next_reservation, len(reservations) + 1)

    def is_exhausted(self, dimension: BudgetDimension | str | None = None) -> bool:
        with self._lock:
            snapshot = self._snapshot_locked()
            if dimension is None:
                return any(
                    bool(getattr(snapshot.remaining, field_name) <= 0)
                    for field_name in (
                        "model_calls",
                        "tool_calls",
                        "total_tokens",
                        "artifact_bytes",
                        "duration_seconds",
                        "cost",
                        "replanning_attempts",
                    )
                )
            selected = BudgetDimension(dimension)
            remaining: Numeric = getattr(snapshot.remaining, _field_name(selected))
            return remaining <= 0

    def _snapshot_locked(self) -> BudgetSnapshot:
        return BudgetSnapshot(budget=self._budget, usage=self._usage, reserved=self._reserved)

    def _next_reservation_id(self) -> str:
        while f"reservation-{self._next_reservation}" in self._reservations:
            self._next_reservation += 1
        reservation_id = f"reservation-{self._next_reservation}"
        self._next_reservation += 1
        return reservation_id

    def _require_reservation_locked(self, reservation: BudgetReservation) -> None:
        recorded = self._reservations.get(reservation.reservation_id)
        if recorded is None:
            raise BudgetReservationUnknownError(
                f"unknown reservation: {reservation.reservation_id}"
            )
        if recorded != reservation.usage:
            raise BudgetReservationMismatchError("reservation usage does not match the ledger")

    def _check_locked(self, charge: BudgetUsage, *, released: BudgetUsage | None = None) -> None:
        effective_reserved = self._reserved if released is None else self._reserved - released
        prospective = self._usage + effective_reserved + charge
        dimensions: tuple[tuple[BudgetDimension, Numeric, Numeric, Numeric], ...] = (
            (
                BudgetDimension.MODEL_CALLS,
                prospective.model_calls,
                self._budget.max_model_calls,
                self._budget.max_model_calls
                - self._usage.model_calls
                - effective_reserved.model_calls,
            ),
            (
                BudgetDimension.TOOL_CALLS,
                prospective.tool_calls,
                self._budget.max_tool_calls,
                self._budget.max_tool_calls
                - self._usage.tool_calls
                - effective_reserved.tool_calls,
            ),
            (
                BudgetDimension.TOTAL_TOKENS,
                prospective.total_tokens,
                self._budget.max_total_tokens,
                self._budget.max_total_tokens
                - self._usage.total_tokens
                - effective_reserved.total_tokens,
            ),
            (
                BudgetDimension.ARTIFACT_BYTES,
                prospective.artifact_bytes,
                self._budget.max_artifact_bytes,
                self._budget.max_artifact_bytes
                - self._usage.artifact_bytes
                - effective_reserved.artifact_bytes,
            ),
            (
                BudgetDimension.DURATION_SECONDS,
                prospective.duration_seconds,
                self._budget.max_duration_seconds,
                self._budget.max_duration_seconds
                - self._usage.duration_seconds
                - effective_reserved.duration_seconds,
            ),
            (
                BudgetDimension.COST,
                prospective.cost,
                self._budget.max_cost,
                self._budget.max_cost - self._usage.cost - effective_reserved.cost,
            ),
            (
                BudgetDimension.REPLANNING_ATTEMPTS,
                prospective.replanning_attempts,
                self._budget.max_replanning_attempts,
                self._budget.max_replanning_attempts
                - self._usage.replanning_attempts
                - effective_reserved.replanning_attempts,
            ),
        )
        for dimension, requested, limit, remaining in dimensions:
            if requested > limit:
                exception_type = _EXCEPTION_BY_DIMENSION.get(dimension, BudgetExhaustedError)
                raise exception_type(dimension, requested, max(0, remaining), limit)

    def _merge_usage(
        self,
        usage: BudgetUsage | None,
        kwargs: dict[str, Numeric],
    ) -> BudgetUsage:
        if usage is not None and not isinstance(usage, BudgetUsage):
            raise TypeError("usage must be a BudgetUsage")
        values: dict[str, Numeric] = {
            "model_calls": 0,
            "tool_calls": 0,
            "total_tokens": 0,
            "artifact_bytes": 0,
            "duration_seconds": 0.0,
            "cost": Decimal("0"),
            "replanning_attempts": 0,
        }
        if usage is not None:
            values = {
                "model_calls": usage.model_calls,
                "tool_calls": usage.tool_calls,
                "total_tokens": usage.total_tokens,
                "artifact_bytes": usage.artifact_bytes,
                "duration_seconds": usage.duration_seconds,
                "cost": usage.cost,
                "replanning_attempts": usage.replanning_attempts,
            }
        aliases = {
            "tokens": "total_tokens",
            "artifact_size": "artifact_bytes",
            "replanning": "replanning_attempts",
        }
        for key, value in kwargs.items():
            normalized_key = aliases.get(key, key)
            if normalized_key not in values:
                raise TypeError(f"unknown budget dimension: {key}")
            if normalized_key == "duration_seconds":
                increment = _nonnegative_duration(value, normalized_key)
                values[normalized_key] = _nonnegative_duration(
                    float(values[normalized_key]) + increment,
                    normalized_key,
                )
            elif normalized_key == "cost":
                increment_decimal = _nonnegative_decimal(value, normalized_key)
                values[normalized_key] = _nonnegative_decimal(
                    Decimal(values[normalized_key]) + increment_decimal,
                    normalized_key,
                )
            else:
                increment_int = _nonnegative_int(value, normalized_key)
                values[normalized_key] = _nonnegative_int(
                    int(values[normalized_key]) + increment_int,
                    normalized_key,
                )
        return BudgetUsage(
            model_calls=int(values["model_calls"]),
            tool_calls=int(values["tool_calls"]),
            total_tokens=int(values["total_tokens"]),
            artifact_bytes=int(values["artifact_bytes"]),
            duration_seconds=float(values["duration_seconds"]),
            cost=Decimal(values["cost"]),
            replanning_attempts=int(values["replanning_attempts"]),
        )


def _field_name(dimension: BudgetDimension) -> str:
    return dimension.value


__all__ = (
    "BUDGET_STATE_VERSION",
    "ArtifactBudgetExhaustedError",
    "BudgetCounterError",
    "BudgetDimension",
    "BudgetError",
    "BudgetExceeded",
    "BudgetExceededError",
    "BudgetExhaustedError",
    "BudgetLedger",
    "BudgetLimitExceededError",
    "BudgetReservation",
    "BudgetReservationMismatchError",
    "BudgetReservationUnknownError",
    "BudgetSnapshot",
    "BudgetStateError",
    "BudgetUsage",
    "CostBudgetExhaustedError",
    "DurationBudgetExhaustedError",
    "ModelCallBudgetExhaustedError",
    "Numeric",
    "ReplanningBudgetExhaustedError",
    "RunBudget",
    "TokenBudgetExhaustedError",
    "ToolCallBudgetExhaustedError",
)
