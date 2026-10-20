//! `BudgetLedger`: usage accounting with reservations, transactional state
//! import/export and thread safety.

use crate::counter::Counter;
use crate::decimal::Decimal;
use crate::dimension::BudgetDimension;
use crate::duration::{normalize_duration, python_float_parse};
use crate::errors::{BudgetError, BudgetValue};
use crate::json::{
    counter_from_integer_text, counter_value, decimal_from_integer_text, json_integer_text,
    number_text,
};
use crate::run_budget::RunBudget;
use crate::usage::{
    BudgetSnapshot, BudgetUsage, ModelCallCharge, ToolCallCharge, UsageDelta, exhaustion,
};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use serde_json::{Map, Value};
use std::sync::{Mutex, MutexGuard};

/// Version of the exported budget state.
pub const BUDGET_STATE_VERSION: i64 = 1;

const USAGE_STATE_KEYS: [&str; 7] = [
    "model_calls",
    "tool_calls",
    "total_tokens",
    "artifact_bytes",
    "duration_seconds",
    "cost",
    "replanning_attempts",
];

const RESERVATION_STATE_KEYS: [&str; 2] = ["reservation_id", "usage"];

const BUDGET_STATE_KEYS: [&str; 6] = [
    "version",
    "budget",
    "usage",
    "reserved",
    "reservations",
    "next_reservation",
];

/// A reservation returned by [`BudgetLedger::reserve`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetReservation {
    reservation_id: String,
    usage: BudgetUsage,
}

impl BudgetReservation {
    /// Construct a reservation value (validated when committed or released).
    #[must_use]
    pub fn new(reservation_id: impl Into<String>, usage: BudgetUsage) -> Self {
        Self {
            reservation_id: reservation_id.into(),
            usage,
        }
    }

    /// The reservation id.
    #[must_use]
    pub fn reservation_id(&self) -> &str {
        &self.reservation_id
    }

    /// The reserved usage.
    #[must_use]
    pub const fn usage(&self) -> &BudgetUsage {
        &self.usage
    }

    /// `BudgetReservation.as_dict()`.
    #[must_use]
    pub fn as_dict(&self) -> Value {
        let mut object = Map::new();
        object.insert(
            "reservation_id".to_owned(),
            Value::String(self.reservation_id.clone()),
        );
        object.insert("usage".to_owned(), self.usage.as_dict());
        Value::Object(object)
    }
}

#[derive(Clone, Debug)]
struct LedgerState {
    budget: RunBudget,
    usage: BudgetUsage,
    reserved: BudgetUsage,
    reservations: Vec<(String, BudgetUsage)>,
    next_reservation: Counter,
}

/// A thread-safe budget ledger mirroring `kala_agent.kernel.budget.BudgetLedger`.
pub struct BudgetLedger {
    inner: Mutex<LedgerState>,
}

impl BudgetLedger {
    /// Create a ledger with the supplied budget (or the Python defaults).
    #[must_use]
    pub fn new(budget: RunBudget) -> Self {
        Self {
            inner: Mutex::new(LedgerState {
                budget,
                usage: BudgetUsage::new(),
                reserved: BudgetUsage::new(),
                reservations: Vec::new(),
                next_reservation: Counter::one(),
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, LedgerState> {
        // Python's RLock cannot be poisoned; recover instead of panicking.
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The configured budget.
    #[must_use]
    pub fn budget(&self) -> RunBudget {
        self.lock().budget.clone()
    }

    /// Accumulated usage.
    #[must_use]
    pub fn usage(&self) -> BudgetUsage {
        self.lock().usage.clone()
    }

    /// Outstanding reserved usage.
    #[must_use]
    pub fn reserved(&self) -> BudgetUsage {
        self.lock().reserved.clone()
    }

    /// Remaining budget (`available`/`remaining` in Python).
    pub fn available(&self) -> Result<BudgetUsage, BudgetError> {
        self.snapshot().remaining()
    }

    /// Alias for [`Self::available`].
    pub fn remaining(&self) -> Result<BudgetUsage, BudgetError> {
        self.available()
    }

    /// A consistent snapshot of budget, usage and reserved usage.
    #[must_use]
    pub fn snapshot(&self) -> BudgetSnapshot {
        let state = self.lock();
        BudgetSnapshot::new(
            state.budget.clone(),
            state.usage.clone(),
            state.reserved.clone(),
        )
    }

    /// Return `true` when the charge fits, `false` on exhaustion or input
    /// validation failure and an error for decimal failures, matching the
    /// Python `can_consume` swallow list (`TypeError`, `ValueError`,
    /// `BudgetExhaustedError`; decimal arithmetic errors propagate).
    pub fn can_consume(&self, charge: &BudgetUsage) -> Result<bool, BudgetError> {
        match self.check(charge) {
            Ok(()) => Ok(true),
            Err(error) => swallow_can_consume(error),
        }
    }

    /// Return `true` when the delta fits.
    pub fn can_consume_delta(&self, delta: &UsageDelta) -> Result<bool, BudgetError> {
        match self.check_delta(delta) {
            Ok(()) => Ok(true),
            Err(error) => swallow_can_consume(error),
        }
    }

    /// Validate a charge against the current ledger without mutating it.
    pub fn check(&self, charge: &BudgetUsage) -> Result<(), BudgetError> {
        let state = self.lock();
        self.check_locked(&state, charge, None)
    }

    /// Validate a delta against the current ledger without mutating it.
    pub fn check_delta(&self, delta: &UsageDelta) -> Result<(), BudgetError> {
        let charge = BudgetUsage::merge(None, delta)?;
        let state = self.lock();
        self.check_locked(&state, &charge, None)
    }

    /// Validate an optional base usage plus delta increments.
    pub fn check_charge(
        &self,
        base: Option<&BudgetUsage>,
        delta: &UsageDelta,
    ) -> Result<(), BudgetError> {
        let charge = BudgetUsage::merge(base, delta)?;
        let state = self.lock();
        self.check_locked(&state, &charge, None)
    }

    /// Alias for [`Self::check`] matching the Python `ensure`.
    pub fn ensure(&self, charge: &BudgetUsage) -> Result<(), BudgetError> {
        self.check(charge)
    }

    /// Charge a full usage value and return the updated usage.
    pub fn consume(&self, charge: &BudgetUsage) -> Result<BudgetUsage, BudgetError> {
        let mut state = self.lock();
        self.check_locked(&state, charge, None)?;
        let updated = state.usage.add(charge)?;
        state.usage = updated;
        Ok(state.usage.clone())
    }

    /// Charge delta increments and return the updated usage.
    pub fn consume_delta(&self, delta: &UsageDelta) -> Result<BudgetUsage, BudgetError> {
        let charge = BudgetUsage::merge(None, delta)?;
        self.consume(&charge)
    }

    /// Alias for [`Self::consume`] matching the Python `charge`/`record`.
    pub fn charge(&self, charge: &BudgetUsage) -> Result<BudgetUsage, BudgetError> {
        self.consume(charge)
    }

    /// Alias for [`Self::consume`] matching the Python `record`.
    pub fn record(&self, charge: &BudgetUsage) -> Result<BudgetUsage, BudgetError> {
        self.consume(charge)
    }

    /// `record_model_call` with the Python defaults (one call).
    pub fn record_model_call(&self, call: ModelCallCharge) -> Result<BudgetUsage, BudgetError> {
        let delta = UsageDelta {
            model_calls: call.count,
            total_tokens: call.tokens,
            duration_seconds: call.duration_seconds,
            cost: call.cost,
            ..UsageDelta::default()
        };
        self.consume_delta(&delta)
    }

    /// `record_model_calls` alias.
    pub fn record_model_calls(&self, call: ModelCallCharge) -> Result<BudgetUsage, BudgetError> {
        self.record_model_call(call)
    }

    /// `record_tool_call` with the Python defaults (one call).
    pub fn record_tool_call(&self, call: ToolCallCharge) -> Result<BudgetUsage, BudgetError> {
        let delta = UsageDelta {
            tool_calls: call.count,
            duration_seconds: call.duration_seconds,
            artifact_bytes: call.artifact_bytes,
            cost: call.cost,
            ..UsageDelta::default()
        };
        self.consume_delta(&delta)
    }

    /// `record_tool_calls` alias.
    pub fn record_tool_calls(&self, call: ToolCallCharge) -> Result<BudgetUsage, BudgetError> {
        self.record_tool_call(call)
    }

    /// Record tokens.
    pub fn record_tokens(&self, tokens: impl Into<Counter>) -> Result<BudgetUsage, BudgetError> {
        self.consume_delta(&UsageDelta::new().total_tokens(tokens))
    }

    /// Record artifact bytes.
    pub fn record_artifact_bytes(
        &self,
        artifact_bytes: impl Into<Counter>,
    ) -> Result<BudgetUsage, BudgetError> {
        self.consume_delta(&UsageDelta::new().artifact_bytes(artifact_bytes))
    }

    /// Record duration seconds.
    pub fn record_duration(&self, duration_seconds: f64) -> Result<BudgetUsage, BudgetError> {
        self.consume_delta(&UsageDelta::new().duration_seconds(duration_seconds))
    }

    /// Record an exact decimal cost.
    pub fn record_cost(&self, cost: Decimal) -> Result<BudgetUsage, BudgetError> {
        self.consume_delta(&UsageDelta::new().cost(cost))
    }

    /// Parse and record a decimal cost.
    pub fn record_cost_str(&self, cost: &str) -> Result<BudgetUsage, BudgetError> {
        self.consume_delta(&UsageDelta::new().cost_str(cost)?)
    }

    /// Record replanning attempts.
    pub fn record_replanning(&self, count: impl Into<Counter>) -> Result<BudgetUsage, BudgetError> {
        self.consume_delta(&UsageDelta::new().replanning_attempts(count))
    }

    /// `record_replanning_attempt` alias.
    pub fn record_replanning_attempt(
        &self,
        count: impl Into<Counter>,
    ) -> Result<BudgetUsage, BudgetError> {
        self.record_replanning(count)
    }

    /// Reserve a full usage value.
    pub fn reserve(&self, usage: &BudgetUsage) -> Result<BudgetReservation, BudgetError> {
        let mut state = self.lock();
        self.check_locked(&state, usage, None)?;
        let reservation_id = next_reservation_id(&mut state);
        state
            .reservations
            .push((reservation_id.clone(), usage.clone()));
        let updated = state.reserved.add(usage)?;
        state.reserved = updated;
        Ok(BudgetReservation::new(reservation_id, usage.clone()))
    }

    /// Reserve delta increments.
    pub fn reserve_delta(&self, delta: &UsageDelta) -> Result<BudgetReservation, BudgetError> {
        let charge = BudgetUsage::merge(None, delta)?;
        self.reserve(&charge)
    }

    /// Commit a reservation using its reserved usage.
    pub fn commit(&self, reservation: &BudgetReservation) -> Result<BudgetUsage, BudgetError> {
        let charge = reservation.usage.clone();
        self.commit_charge(reservation, &charge)
    }

    /// Commit a reservation charging a different actual usage.
    pub fn commit_with(
        &self,
        reservation: &BudgetReservation,
        actual: &BudgetUsage,
    ) -> Result<BudgetUsage, BudgetError> {
        self.commit_charge(reservation, actual)
    }

    /// Commit a reservation charging delta increments (`commit(..., **kwargs)`).
    pub fn commit_delta(
        &self,
        reservation: &BudgetReservation,
        delta: &UsageDelta,
    ) -> Result<BudgetUsage, BudgetError> {
        let charge = BudgetUsage::merge(None, delta)?;
        self.commit_charge(reservation, &charge)
    }

    fn commit_charge(
        &self,
        reservation: &BudgetReservation,
        charge: &BudgetUsage,
    ) -> Result<BudgetUsage, BudgetError> {
        let mut state = self.lock();
        require_reservation(&state, reservation)?;
        self.check_locked(&state, charge, Some(&reservation.usage))?;
        remove_reservation(&mut state, reservation.reservation_id());
        let reserved = state.reserved.sub(&reservation.usage)?;
        state.reserved = reserved;
        let updated = state.usage.add(charge)?;
        state.usage = updated;
        Ok(state.usage.clone())
    }

    /// Release a reservation and return the remaining reserved usage.
    pub fn release(&self, reservation: &BudgetReservation) -> Result<BudgetUsage, BudgetError> {
        let mut state = self.lock();
        require_reservation(&state, reservation)?;
        let recorded = find_reservation(&state, reservation.reservation_id())
            .cloned()
            .ok_or_else(|| BudgetError::ReservationUnknown {
                id: reservation.reservation_id().to_owned(),
            })?;
        let remaining = state.reserved.sub(&recorded)?;
        remove_reservation(&mut state, reservation.reservation_id());
        state.reserved = remaining;
        Ok(state.reserved.clone())
    }

    /// Snapshot of outstanding reservations in insertion order.
    #[must_use]
    pub fn reservations(&self) -> Vec<BudgetReservation> {
        self.lock()
            .reservations
            .iter()
            .map(|(id, usage)| BudgetReservation::new(id.clone(), usage.clone()))
            .collect()
    }

    /// `BudgetLedger.export_state()`.
    #[must_use]
    pub fn export_state(&self) -> Value {
        let state = self.lock();
        let reservations: Vec<Value> = state
            .reservations
            .iter()
            .map(|(id, usage)| {
                let mut object = Map::new();
                object.insert("reservation_id".to_owned(), Value::String(id.clone()));
                object.insert("usage".to_owned(), usage.as_dict());
                Value::Object(object)
            })
            .collect();
        let mut object = Map::new();
        object.insert("version".to_owned(), Value::from(BUDGET_STATE_VERSION));
        object.insert("budget".to_owned(), state.budget.as_dict());
        object.insert("usage".to_owned(), state.usage.as_dict());
        object.insert("reserved".to_owned(), state.reserved.as_dict());
        object.insert("reservations".to_owned(), Value::Array(reservations));
        object.insert(
            "next_reservation".to_owned(),
            counter_value(&state.next_reservation),
        );
        Value::Object(object)
    }

    /// `BudgetLedger.import_state()`: all validation happens before any
    /// mutation, so a rejected state leaves the ledger untouched.
    pub fn import_state(&self, state: &Value) -> Result<(), BudgetError> {
        let parsed = parse_state(state)?;
        let mut guard = self.lock();
        *guard = parsed;
        Ok(())
    }

    /// True when any (or the named) dimension has no remaining budget.
    pub fn is_exhausted(&self, dimension: Option<BudgetDimension>) -> Result<bool, BudgetError> {
        let snapshot = self.snapshot();
        let remaining = snapshot.remaining()?;
        match dimension {
            None => Ok(BudgetDimension::ALL
                .iter()
                .any(|dimension| remaining_is_exhausted(&remaining, *dimension))),
            Some(selected) => Ok(remaining_is_exhausted(&remaining, selected)),
        }
    }

    /// `is_exhausted` accepting the Python string form of a dimension.
    pub fn is_exhausted_named(&self, dimension: &str) -> Result<bool, BudgetError> {
        let selected: BudgetDimension = dimension.parse()?;
        self.is_exhausted(Some(selected))
    }

    fn check_locked(
        &self,
        state: &LedgerState,
        charge: &BudgetUsage,
        released: Option<&BudgetUsage>,
    ) -> Result<(), BudgetError> {
        let effective_reserved = match released {
            Some(released) => state.reserved.sub(released)?,
            None => state.reserved.clone(),
        };
        let prospective = state.usage.add(&effective_reserved)?.add(charge)?;

        let budget = &state.budget;
        let usage = &state.usage;
        let reserved = &effective_reserved;

        let model_remaining = counter_remaining(
            budget.max_model_calls(),
            usage.model_calls(),
            reserved.model_calls(),
        );
        let tool_remaining = counter_remaining(
            budget.max_tool_calls(),
            usage.tool_calls(),
            reserved.tool_calls(),
        );
        let token_remaining = counter_remaining(
            budget.max_total_tokens(),
            usage.total_tokens(),
            reserved.total_tokens(),
        );
        let artifact_remaining = counter_remaining(
            budget.max_artifact_bytes(),
            usage.artifact_bytes(),
            reserved.artifact_bytes(),
        );
        let duration_remaining_raw =
            budget.max_duration_seconds() - usage.duration_seconds() - reserved.duration_seconds();
        let cost_remaining_raw = budget.max_cost().sub(usage.cost())?.sub(reserved.cost())?;
        let replanning_remaining = counter_remaining(
            budget.max_replanning_attempts(),
            usage.replanning_attempts(),
            reserved.replanning_attempts(),
        );

        let dimensions: [(
            &BudgetDimension,
            bool,
            BudgetValue,
            BudgetValue,
            BudgetValue,
        ); 7] = [
            (
                &BudgetDimension::ModelCalls,
                prospective.model_calls() > budget.max_model_calls(),
                BudgetValue::Integer(prospective.model_calls().clone()),
                BudgetValue::Integer(model_remaining),
                BudgetValue::Integer(budget.max_model_calls().clone()),
            ),
            (
                &BudgetDimension::ToolCalls,
                prospective.tool_calls() > budget.max_tool_calls(),
                BudgetValue::Integer(prospective.tool_calls().clone()),
                BudgetValue::Integer(tool_remaining),
                BudgetValue::Integer(budget.max_tool_calls().clone()),
            ),
            (
                &BudgetDimension::TotalTokens,
                prospective.total_tokens() > budget.max_total_tokens(),
                BudgetValue::Integer(prospective.total_tokens().clone()),
                BudgetValue::Integer(token_remaining),
                BudgetValue::Integer(budget.max_total_tokens().clone()),
            ),
            (
                &BudgetDimension::ArtifactBytes,
                prospective.artifact_bytes() > budget.max_artifact_bytes(),
                BudgetValue::Integer(prospective.artifact_bytes().clone()),
                BudgetValue::Integer(artifact_remaining),
                BudgetValue::Integer(budget.max_artifact_bytes().clone()),
            ),
            (
                &BudgetDimension::DurationSeconds,
                prospective.duration_seconds() > budget.max_duration_seconds(),
                BudgetValue::Float(prospective.duration_seconds()),
                clamped_duration_value(duration_remaining_raw),
                BudgetValue::Float(budget.max_duration_seconds()),
            ),
            (
                &BudgetDimension::Cost,
                prospective.cost() > budget.max_cost(),
                BudgetValue::Decimal(prospective.cost().clone()),
                clamped_cost_value(cost_remaining_raw),
                BudgetValue::Decimal(budget.max_cost().clone()),
            ),
            (
                &BudgetDimension::ReplanningAttempts,
                prospective.replanning_attempts() > budget.max_replanning_attempts(),
                BudgetValue::Integer(prospective.replanning_attempts().clone()),
                BudgetValue::Integer(replanning_remaining),
                BudgetValue::Integer(budget.max_replanning_attempts().clone()),
            ),
        ];

        for (dimension, exceeded, requested, remaining, limit) in dimensions {
            if exceeded {
                return Err(exhaustion(*dimension, requested, remaining, limit));
            }
        }
        Ok(())
    }
}

impl Default for BudgetLedger {
    fn default() -> Self {
        Self::new(RunBudget::new())
    }
}

impl std::fmt::Debug for BudgetLedger {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BudgetLedger")
            .field("budget", &self.budget())
            .field("usage", &self.usage())
            .field("reserved", &self.reserved())
            .field("reservations", &self.reservations())
            .finish_non_exhaustive()
    }
}

fn swallow_can_consume(error: BudgetError) -> Result<bool, BudgetError> {
    match error {
        BudgetError::Exhausted(_)
        | BudgetError::Validation(_)
        | BudgetError::CounterUnderflow
        | BudgetError::Counter(_)
        | BudgetError::DecimalParse(_)
        | BudgetError::Dimension(_) => Ok(false),
        other => Err(other),
    }
}

fn remaining_is_exhausted(usage: &BudgetUsage, dimension: BudgetDimension) -> bool {
    match dimension {
        BudgetDimension::ModelCalls => usage.model_calls().is_zero(),
        BudgetDimension::ToolCalls => usage.tool_calls().is_zero(),
        BudgetDimension::TotalTokens => usage.total_tokens().is_zero(),
        BudgetDimension::ArtifactBytes => usage.artifact_bytes().is_zero(),
        BudgetDimension::DurationSeconds => usage.duration_seconds() <= 0.0,
        BudgetDimension::Cost => *usage.cost() <= Decimal::zero(),
        BudgetDimension::ReplanningAttempts => usage.replanning_attempts().is_zero(),
    }
}

fn counter_remaining(max: &Counter, usage: &Counter, reserved: &Counter) -> Counter {
    let value = BigInt::from(max.as_biguint().clone())
        - BigInt::from(usage.as_biguint().clone())
        - BigInt::from(reserved.as_biguint().clone());
    if value <= BigInt::from(0u32) {
        Counter::zero()
    } else {
        Counter::from(value.magnitude().clone())
    }
}

fn clamped_duration_value(raw: f64) -> BudgetValue {
    // Python `max(0, remaining)`: an int zero wins over any non-positive float.
    if raw > 0.0 {
        BudgetValue::Float(raw)
    } else {
        BudgetValue::Integer(Counter::zero())
    }
}

fn clamped_cost_value(raw: Decimal) -> BudgetValue {
    if raw > Decimal::zero() {
        BudgetValue::Decimal(raw)
    } else {
        BudgetValue::Decimal(Decimal::zero())
    }
}

fn next_reservation_id(state: &mut LedgerState) -> String {
    loop {
        let candidate = format!("reservation-{}", state.next_reservation);
        if find_reservation(state, &candidate).is_some() {
            state.next_reservation = state.next_reservation.increment();
            continue;
        }
        state.next_reservation = state.next_reservation.increment();
        return candidate;
    }
}

fn find_reservation<'a>(state: &'a LedgerState, id: &str) -> Option<&'a BudgetUsage> {
    state
        .reservations
        .iter()
        .find(|(candidate, _)| candidate == id)
        .map(|(_, usage)| usage)
}

fn remove_reservation(state: &mut LedgerState, id: &str) {
    state.reservations.retain(|(candidate, _)| candidate != id);
}

fn require_reservation(
    state: &LedgerState,
    reservation: &BudgetReservation,
) -> Result<(), BudgetError> {
    let recorded = find_reservation(state, reservation.reservation_id()).ok_or_else(|| {
        BudgetError::ReservationUnknown {
            id: reservation.reservation_id().to_owned(),
        }
    })?;
    if recorded != reservation.usage() {
        return Err(BudgetError::ReservationMismatch {
            id: reservation.reservation_id().to_owned(),
        });
    }
    Ok(())
}

fn parse_state(state: &Value) -> Result<LedgerState, BudgetError> {
    let object = state
        .as_object()
        .ok_or_else(|| BudgetError::State("state must be a mapping".to_owned()))?;
    let mut unexpected: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !BUDGET_STATE_KEYS.contains(key))
        .collect();
    unexpected.sort_unstable();
    if !unexpected.is_empty() {
        return Err(BudgetError::State(format!(
            "state has unexpected keys: {}",
            unexpected.join(", ")
        )));
    }

    let version_value = object
        .get("version")
        .ok_or_else(|| BudgetError::State("state.version must be an integer".to_owned()))?;
    let version_text = json_integer_text(version_value)
        .ok_or_else(|| BudgetError::State("state.version must be an integer".to_owned()))?;
    let version = BigInt::parse_bytes(version_text.as_bytes(), 10)
        .ok_or_else(|| BudgetError::State("state.version must be an integer".to_owned()))?;
    if version != BigInt::from(BUDGET_STATE_VERSION) {
        return Err(BudgetError::UnsupportedVersion {
            version: version.to_string(),
        });
    }

    let empty_object = Value::Object(Map::new());
    let budget_value = object.get("budget").unwrap_or(&empty_object);
    let budget_object = budget_value
        .as_object()
        .ok_or_else(|| BudgetError::State("state.budget must be a mapping".to_owned()))?;
    let mut unexpected_budget: Vec<&str> = budget_object
        .keys()
        .map(String::as_str)
        .filter(|key| {
            ![
                "max_model_calls",
                "max_tool_calls",
                "max_total_tokens",
                "max_artifact_bytes",
                "max_duration_seconds",
                "max_cost",
                "max_replanning_attempts",
            ]
            .contains(key)
        })
        .collect();
    unexpected_budget.sort_unstable();
    if !unexpected_budget.is_empty() {
        return Err(BudgetError::State(format!(
            "state.budget has unexpected keys: {}",
            unexpected_budget.join(", ")
        )));
    }
    let budget = RunBudget::from_json(&Value::Object(budget_object.clone()))
        .map_err(|error| BudgetError::State(format!("state.budget is invalid: {error}")))?;

    let usage = usage_from_state(object.get("usage").unwrap_or(&empty_object), "state.usage")?;
    let reserved = usage_from_state(
        object.get("reserved").unwrap_or(&empty_object),
        "state.reserved",
    )?;

    let reservations_value = object
        .get("reservations")
        .cloned()
        .unwrap_or(Value::Array(Vec::new()));
    let reservation_array = reservations_value
        .as_array()
        .ok_or_else(|| BudgetError::State("state.reservations must be a sequence".to_owned()))?;
    let mut reservations: Vec<(String, BudgetUsage)> = Vec::new();
    for item in reservation_array {
        let (id, reservation_usage) = reservation_from_state(item)?;
        if reservations.iter().any(|(existing, _)| existing == &id) {
            return Err(BudgetError::DuplicateReservation { id });
        }
        reservations.push((id, reservation_usage));
    }

    let mut expected = BudgetUsage::new();
    for (_, reservation_usage) in &reservations {
        expected = expected.add(reservation_usage)?;
    }
    if expected != reserved {
        return Err(BudgetError::State(
            "state.reserved does not match the reservation total".to_owned(),
        ));
    }

    let default_next = Counter::from(reservations.len() + 1);
    let next_reservation = match object.get("next_reservation") {
        None => default_next,
        Some(value) => {
            let text = json_integer_text(value).ok_or_else(|| {
                BudgetError::State("state.next_reservation must be an integer".to_owned())
            })?;
            let parsed = num_bigint::BigInt::parse_bytes(text.as_bytes(), 10).ok_or_else(|| {
                BudgetError::State("state.next_reservation must be an integer".to_owned())
            })?;
            if parsed < BigInt::from(1u32) {
                return Err(BudgetError::State(
                    "state.next_reservation must be positive".to_owned(),
                ));
            }
            Counter::from(parsed.magnitude().clone())
        }
    };
    let minimum = Counter::from(reservations.len() + 1);
    let next_reservation = if next_reservation > minimum {
        next_reservation
    } else {
        minimum
    };

    Ok(LedgerState {
        budget,
        usage,
        reserved,
        reservations,
        next_reservation,
    })
}

fn usage_from_state(value: &Value, field: &str) -> Result<BudgetUsage, BudgetError> {
    let object = value
        .as_object()
        .ok_or_else(|| BudgetError::State(format!("{field} must be a mapping")))?;
    let mut unexpected: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !USAGE_STATE_KEYS.contains(key))
        .collect();
    unexpected.sort_unstable();
    if !unexpected.is_empty() {
        return Err(BudgetError::State(format!(
            "{field} has unexpected keys: {}",
            unexpected.join(", ")
        )));
    }
    usage_fields(object, field)
        .map_err(|message| BudgetError::State(format!("{field} is invalid: {message}")))
}

fn usage_fields(object: &Map<String, Value>, field: &str) -> Result<BudgetUsage, String> {
    let mut usage = BudgetUsage::new();
    if let Some(value) = object.get("model_calls") {
        usage = usage.with_model_calls(state_counter(value, &format!("{field}.model_calls"))?);
    }
    if let Some(value) = object.get("tool_calls") {
        usage = usage.with_tool_calls(state_counter(value, &format!("{field}.tool_calls"))?);
    }
    if let Some(value) = object.get("total_tokens") {
        usage = usage.with_total_tokens(state_counter(value, &format!("{field}.total_tokens"))?);
    }
    if let Some(value) = object.get("artifact_bytes") {
        usage =
            usage.with_artifact_bytes(state_counter(value, &format!("{field}.artifact_bytes"))?);
    }
    if let Some(value) = object.get("duration_seconds") {
        usage = usage
            .try_with_duration_seconds(state_duration(value, &format!("{field}.duration_seconds"))?)
            .map_err(|error| error.to_string())?;
    }
    if let Some(value) = object.get("cost") {
        let cost = state_cost(value, &format!("{field}.cost"))?;
        usage = usage
            .try_with_cost(cost)
            .map_err(|error| error.to_string())?;
    }
    if let Some(value) = object.get("replanning_attempts") {
        usage = usage.with_replanning_attempts(state_counter(
            value,
            &format!("{field}.replanning_attempts"),
        )?);
    }
    Ok(usage)
}

fn state_counter(value: &Value, field: &str) -> Result<Counter, String> {
    let name = field.rsplit('.').next().unwrap_or(field);
    let text = json_integer_text(value).ok_or_else(|| format!("{field} must be an integer"))?;
    if text.starts_with('-') && text[1..].bytes().any(|byte| byte != b'0') {
        return Err(format!("{name} must be nonnegative"));
    }
    counter_from_integer_text(text).map_err(|_| format!("{name} must be nonnegative"))
}

fn state_duration(value: &Value, field: &str) -> Result<f64, String> {
    let parsed = match value {
        Value::String(text) => {
            python_float_parse(text).map_err(|_| format!("{field} must be numeric"))?
        }
        Value::Number(_) => {
            let text = number_text(value).ok_or_else(|| format!("{field} must be numeric"))?;
            if crate::json::is_integer_text(text) {
                let integer = BigInt::parse_bytes(text.as_bytes(), 10)
                    .ok_or_else(|| format!("{field} must be numeric"))?;
                integer
                    .to_f64()
                    .ok_or_else(|| "duration_seconds must be finite and nonnegative".to_owned())?
            } else {
                text.parse::<f64>()
                    .map_err(|_| format!("{field} must be numeric"))?
            }
        }
        _ => return Err(format!("{field} must be numeric")),
    };
    if !parsed.is_finite() || parsed < 0.0 {
        return Err("duration_seconds must be finite and nonnegative".to_owned());
    }
    normalize_duration(parsed)
        .map_err(|_| "duration_seconds must be finite and nonnegative".to_owned())
}

fn state_cost(value: &Value, field: &str) -> Result<Decimal, String> {
    let parsed = match value {
        Value::String(text) => Decimal::parse(text)
            .map_err(|_| "cost must be a finite nonnegative decimal".to_owned())?,
        Value::Number(_) => {
            let text = number_text(value).ok_or_else(|| format!("{field} must be numeric"))?;
            if crate::json::is_integer_text(text) {
                decimal_from_integer_text(text)
                    .map_err(|_| "cost must be a finite nonnegative decimal".to_owned())?
            } else {
                let float = text
                    .parse::<f64>()
                    .map_err(|_| format!("{field} must be numeric"))?;
                Decimal::from_python_float(float)
                    .map_err(|_| "cost must be a finite nonnegative decimal".to_owned())?
            }
        }
        _ => return Err(format!("{field} must be numeric")),
    };
    if parsed.is_negative() && !parsed.is_zero() {
        return Err("cost must be a finite nonnegative decimal".to_owned());
    }
    Ok(parsed)
}

fn reservation_from_state(value: &Value) -> Result<(String, BudgetUsage), BudgetError> {
    let object = value
        .as_object()
        .ok_or_else(|| BudgetError::State("reservation must be a mapping".to_owned()))?;
    let mut unexpected: Vec<&str> = object
        .keys()
        .map(String::as_str)
        .filter(|key| !RESERVATION_STATE_KEYS.contains(key))
        .collect();
    unexpected.sort_unstable();
    if !unexpected.is_empty() {
        return Err(BudgetError::State(format!(
            "reservation has unexpected keys: {}",
            unexpected.join(", ")
        )));
    }
    let raw_id = match object.get("reservation_id") {
        Some(Value::String(text)) => text.clone(),
        _ => {
            return Err(BudgetError::State(
                "reservation.reservation_id must be a nonempty bounded string".to_owned(),
            ));
        }
    };
    let trimmed = raw_id
        .trim_matches(|character: char| character.is_whitespace())
        .to_owned();
    if trimmed.is_empty() || raw_id.chars().count() > 256 {
        return Err(BudgetError::State(
            "reservation.reservation_id must be a nonempty bounded string".to_owned(),
        ));
    }
    let empty_object = Value::Object(Map::new());
    let usage = usage_from_state(
        object.get("usage").unwrap_or(&empty_object),
        "reservation.usage",
    )?;
    Ok((trimmed, usage))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledger() -> BudgetLedger {
        BudgetLedger::new(
            RunBudget::new()
                .with_max_tool_calls(2u32)
                .with_max_total_tokens(50u32),
        )
    }

    #[test]
    fn reserve_commit_and_release_follow_python_flow() {
        let ledger = ledger();
        let reservation = ledger
            .reserve_delta(&UsageDelta::new().tool_calls(1u32))
            .unwrap();
        assert_eq!(reservation.reservation_id(), "reservation-1");
        assert_eq!(ledger.reserved().tool_calls(), &Counter::from(1u32));
        ledger.commit(&reservation).unwrap();
        assert_eq!(ledger.usage().tool_calls(), &Counter::from(1u32));
        assert_eq!(ledger.reserved(), BudgetUsage::new());
        assert_eq!(
            ledger.release(&reservation).unwrap_err(),
            BudgetError::ReservationUnknown {
                id: "reservation-1".to_owned()
            }
        );
    }

    #[test]
    fn commit_with_actual_usage_checks_released_reservation() {
        let ledger = ledger();
        let reservation = ledger
            .reserve_delta(&UsageDelta::new().total_tokens(10u32))
            .unwrap();
        let error = ledger
            .commit_with(&reservation, &BudgetUsage::new().with_total_tokens(99u32))
            .unwrap_err();
        assert_eq!(
            error.exhausted_dimension(),
            Some(BudgetDimension::TotalTokens)
        );
        assert_eq!(ledger.reservations(), vec![reservation.clone()]);
        ledger
            .commit_with(&reservation, &BudgetUsage::new().with_total_tokens(4u32))
            .unwrap();
        assert_eq!(ledger.usage().total_tokens(), &Counter::from(4u32));
        assert_eq!(ledger.reserved().total_tokens(), &Counter::zero());
    }

    #[test]
    fn import_is_transactional_on_invalid_state() {
        let ledger = ledger();
        let reservation = ledger
            .reserve_delta(&UsageDelta::new().tool_calls(1u32))
            .unwrap();
        let valid = ledger.export_state();
        let malformed = [
            Value::Array(Vec::new()),
            serde_json::json!({"version": 99}),
            serde_json::json!({"version": "1"}),
            serde_json::json!({"version": 1, "unexpected": true}),
            serde_json::json!({"version": 1, "budget": {"max_tool_calls": -1}}),
            serde_json::json!({"version": 1, "usage": {"tool_calls": -1}}),
            serde_json::json!({"version": 1, "usage": {"cost": "free"}}),
            serde_json::json!({"version": 1, "reservations": [{"reservation_id": "r", "usage": {}, "extra": 1}]}),
            serde_json::json!({"version": 1, "reservations": [{"reservation_id": "", "usage": {}}]}),
            serde_json::json!({"version": 1, "next_reservation": 0}),
        ];
        for payload in malformed {
            assert!(ledger.import_state(&payload).is_err(), "{payload}");
            assert_eq!(ledger.export_state(), valid);
        }
        let restored = BudgetLedger::new(RunBudget::new());
        restored.import_state(&valid).unwrap();
        assert_eq!(restored.reservations(), vec![reservation]);
    }

    #[test]
    fn import_rejects_duplicate_reservations_and_reserved_mismatch() {
        let ledger = ledger();
        let duplicate = serde_json::json!({
            "version": 1,
            "reservations": [
                {"reservation_id": "r", "usage": {}},
                {"reservation_id": " r ", "usage": {}}
            ]
        });
        assert!(matches!(
            ledger.import_state(&duplicate).unwrap_err(),
            BudgetError::DuplicateReservation { .. }
        ));
        let mismatch = serde_json::json!({
            "version": 1,
            "reserved": {"cost": "0.30"},
            "reservations": [{"reservation_id": "r", "usage": {"cost": "0.3"}}]
        });
        assert!(
            ledger.import_state(&mismatch).is_ok(),
            "scale-insensitive equality"
        );
        let mismatch = serde_json::json!({
            "version": 1,
            "reserved": {"cost": "0.30"},
            "reservations": [{"reservation_id": "r", "usage": {"cost": "0.2"}}]
        });
        assert!(ledger.import_state(&mismatch).is_err());
    }

    #[test]
    fn reserve_ids_skip_imported_collisions() {
        let ledger = ledger();
        let state = serde_json::json!({
            "version": 1,
            "budget": {"max_tool_calls": 10},
            "reservations": [
                {"reservation_id": "reservation-1", "usage": {}},
                {"reservation_id": "reservation-2", "usage": {}}
            ],
            "next_reservation": 1
        });
        ledger.import_state(&state).unwrap();
        let reservation = ledger
            .reserve_delta(&UsageDelta::new().tool_calls(1u32))
            .unwrap();
        assert_eq!(reservation.reservation_id(), "reservation-3");
    }
}
