//! Native ledger semantics: reservations, state tamper/rollback, arbitrary
//! counters, exhaustion priority and thread safety.

use kala_budget::{
    BudgetDimension, BudgetError, BudgetLedger, BudgetReservation, BudgetUsage, Counter, Decimal,
    ModelCallCharge, RunBudget, ToolCallCharge, UsageDelta,
};
use serde_json::{Value, json};
use std::sync::Arc;
use std::thread;

mod common;

fn ledger_with(tool_calls: u64, tokens: u64) -> BudgetLedger {
    BudgetLedger::new(
        RunBudget::new()
            .with_max_tool_calls(Counter::from(tool_calls))
            .with_max_total_tokens(Counter::from(tokens)),
    )
}

#[test]
fn default_budget_exhausts_cost_immediately() {
    let ledger = BudgetLedger::default();
    assert!(ledger.is_exhausted(None).unwrap());
    assert!(ledger.is_exhausted(Some(BudgetDimension::Cost)).unwrap());
    assert!(
        !ledger
            .is_exhausted(Some(BudgetDimension::ReplanningAttempts))
            .unwrap()
    );
    let error = ledger
        .record_cost(Decimal::parse("1").unwrap())
        .unwrap_err();
    assert_eq!(error.contract_error_name(), "CostBudgetExhaustedError");
    assert_eq!(ledger.usage().cost().to_python_string(), "0");
    assert_eq!(ledger.available().unwrap().cost().to_python_string(), "0");
}

#[test]
fn exhaustion_reports_the_first_exceeded_dimension() {
    let ledger = BudgetLedger::new(
        RunBudget::new()
            .with_max_model_calls(Counter::from(1u32))
            .with_max_tool_calls(Counter::from(1u32))
            .with_max_total_tokens(Counter::from(1_000u32)),
    );
    let error = ledger
        .consume_delta(
            &UsageDelta::new()
                .model_calls(2u32)
                .tool_calls(2u32)
                .total_tokens(2u32),
        )
        .unwrap_err();
    assert_eq!(
        error.exhausted_dimension(),
        Some(BudgetDimension::ModelCalls)
    );

    let error = ledger
        .consume_delta(&UsageDelta::new().tool_calls(2u32).total_tokens(2u32))
        .unwrap_err();
    assert_eq!(
        error.exhausted_dimension(),
        Some(BudgetDimension::ToolCalls)
    );

    let error = ledger
        .consume_delta(&UsageDelta::new().total_tokens(2_000u32))
        .unwrap_err();
    assert_eq!(
        error.exhausted_dimension(),
        Some(BudgetDimension::TotalTokens)
    );
    // Every rejected charge left the ledger untouched.
    assert_eq!(ledger.usage(), BudgetUsage::new());
}

#[test]
fn arbitrary_size_counters_never_wrap_or_saturate() {
    let huge = Counter::parse("10000000000000000000000000000000000000000").unwrap();
    let ledger = BudgetLedger::new(RunBudget::new().with_max_model_calls(huge.clone()));
    let usage = ledger
        .consume_delta(
            &UsageDelta::new()
                .model_calls(Counter::parse("9999999999999999999999999999999999999999").unwrap()),
        )
        .unwrap();
    assert_eq!(
        usage.model_calls().to_decimal_string(),
        "9999999999999999999999999999999999999999"
    );
    let error = ledger
        .consume_delta(&UsageDelta::new().model_calls(2u32))
        .unwrap_err();
    assert_eq!(
        error.exhausted_dimension(),
        Some(BudgetDimension::ModelCalls)
    );
    assert_eq!(
        ledger.usage().model_calls().to_decimal_string(),
        "9999999999999999999999999999999999999999"
    );
    // The error message carries the exact arbitrary-precision numbers.
    assert_eq!(
        error.to_string(),
        "budget exhausted for model_calls: \
         requested 10000000000000000000000000000000000000001, \
         remaining 1, \
         limit 10000000000000000000000000000000000000000"
    );
}

#[test]
fn reservations_release_in_any_order_and_durations_stay_exact() {
    let ledger = BudgetLedger::new(
        RunBudget::new()
            .try_with_max_duration_seconds(10.0)
            .unwrap(),
    );
    let held: Vec<BudgetReservation> = [0.1, 0.2, 0.3, 1e-7]
        .iter()
        .map(|value| {
            ledger
                .reserve_delta(&UsageDelta::new().duration_seconds(*value))
                .unwrap()
        })
        .collect();
    for index in [1usize, 3, 0, 2] {
        ledger.release(&held[index]).unwrap();
    }
    assert_eq!(ledger.reserved(), BudgetUsage::new());
    assert_eq!(ledger.reserved().duration_seconds(), 0.0);

    let first = ledger
        .reserve_delta(&UsageDelta::new().duration_seconds(0.1))
        .unwrap();
    let second = ledger
        .reserve_delta(&UsageDelta::new().duration_seconds(0.2))
        .unwrap();
    let restored = BudgetLedger::default();
    restored.import_state(&ledger.export_state()).unwrap();
    let restored_second = restored
        .reservations()
        .into_iter()
        .find(|reservation| reservation.reservation_id() == second.reservation_id())
        .unwrap();
    let restored_first = restored
        .reservations()
        .into_iter()
        .find(|reservation| reservation.reservation_id() == first.reservation_id())
        .unwrap();
    restored.release(&restored_second).unwrap();
    restored.release(&restored_first).unwrap();
    assert_eq!(restored.reserved(), BudgetUsage::new());
    let sum = BudgetUsage::new()
        .try_with_duration_seconds(0.1)
        .unwrap()
        .add(&BudgetUsage::new().try_with_duration_seconds(0.2).unwrap())
        .unwrap();
    assert_eq!(sum.duration_seconds(), 0.3);
}

#[test]
fn commit_actual_usage_releases_the_reservation_exactly() {
    let ledger = ledger_with(1, 10);
    let reservation = ledger
        .reserve_delta(&UsageDelta::new().tool_calls(1u32).total_tokens(4u32))
        .unwrap();
    let snapshot = ledger.snapshot();
    let error = ledger
        .commit_delta(
            &reservation,
            &UsageDelta::new().tool_calls(1u32).total_tokens(99u32),
        )
        .unwrap_err();
    assert_eq!(
        error.exhausted_dimension(),
        Some(BudgetDimension::TotalTokens)
    );
    assert_eq!(ledger.snapshot(), snapshot);
    assert_eq!(ledger.reservations(), vec![reservation.clone()]);

    let usage = ledger
        .commit_delta(&reservation, &UsageDelta::new().total_tokens(3u32))
        .unwrap();
    assert_eq!(usage.total_tokens(), &Counter::from(3u32));
    assert_eq!(ledger.reserved(), BudgetUsage::new());
    assert_eq!(
        ledger.release(&reservation).unwrap_err(),
        BudgetError::ReservationUnknown {
            id: reservation.reservation_id().to_owned()
        }
    );
}

#[test]
fn state_import_is_transactional_for_every_tamper_shape() {
    let ledger = ledger_with(2, 50);
    let reservation = ledger
        .reserve_delta(&UsageDelta::new().tool_calls(1u32))
        .unwrap();
    let valid = ledger.export_state();

    let malformed: Vec<(&str, Value)> = vec![
        ("not-a-mapping", json!([])),
        ("version", json!({"version": 99})),
        ("version-type", json!({"version": "1"})),
        ("version-missing", json!({})),
        ("unknown-key", json!({"version": 1, "unexpected": true})),
        ("budget-shape", json!({"version": 1, "budget": "none"})),
        ("budget-key", json!({"version": 1, "budget": {"extra": 1}})),
        (
            "budget-negative",
            json!({"version": 1, "budget": {"max_tool_calls": -1}}),
        ),
        (
            "budget-float",
            json!({"version": 1, "budget": {"max_tool_calls": 1.0}}),
        ),
        ("usage-shape", json!({"version": 1, "usage": []})),
        (
            "usage-negative",
            json!({"version": 1, "usage": {"tool_calls": -1}}),
        ),
        (
            "usage-unknown",
            json!({"version": 1, "usage": {"extra": 1}}),
        ),
        (
            "usage-type",
            json!({"version": 1, "usage": {"cost": "free"}}),
        ),
        (
            "reserved-mismatch",
            json!({"version": 1, "reserved": {"tool_calls": 5}}),
        ),
        (
            "reservation-key",
            json!({"version": 1, "reservations": [{"reservation_id": "r1", "usage": {}, "extra": 1}]}),
        ),
        (
            "reservation-id",
            json!({"version": 1, "reservations": [{"reservation_id": "", "usage": {}}]}),
        ),
        (
            "reservation-id-long",
            json!({"version": 1, "reservations": [{"reservation_id": "x".repeat(257), "usage": {}}]}),
        ),
        (
            "reservation-duplicate",
            json!({"version": 1, "reservations": [valid["reservations"][0], valid["reservations"][0]]}),
        ),
        (
            "next-reservation",
            json!({"version": 1, "next_reservation": 0}),
        ),
        (
            "next-reservation-type",
            json!({"version": 1, "next_reservation": "1"}),
        ),
    ];
    for (label, payload) in malformed {
        let error = ledger
            .import_state(&payload)
            .expect_err(&format!("{label} must be rejected"));
        assert!(
            matches!(error.contract_error_name(), "BudgetStateError"),
            "{label}: {error}"
        );
        common::assert_json_eq(&ledger.export_state(), &valid, label);
    }

    let empty = BudgetLedger::default();
    empty.import_state(&valid).unwrap();
    assert_eq!(empty.reservations(), vec![reservation.clone()]);
    let replacement = empty
        .reserve_delta(&UsageDelta::new().tool_calls(1u32))
        .unwrap();
    assert_ne!(replacement.reservation_id(), reservation.reservation_id());
}

#[test]
fn import_accepts_scale_variants_and_huge_counters() {
    let ledger = BudgetLedger::default();
    let huge = "12345678901234567890123456789012345678901234567890";
    let huge_number: Value = serde_json::from_str(huge).expect("JSON number");
    let state = json!({
        "version": 1,
        "budget": {"max_tool_calls": huge_number, "max_cost": "1.0"},
        "usage": {"tool_calls": huge_number, "cost": "0.30", "duration_seconds": 1.5},
        "reserved": {"cost": "0.0"},
        "reservations": [{"reservation_id": "r", "usage": {"cost": "0"}}],
        "next_reservation": huge_number
    });
    ledger.import_state(&state).unwrap();
    assert_eq!(ledger.usage().tool_calls().to_decimal_string(), huge);
    assert_eq!(ledger.usage().cost().to_python_string(), "0.30");
    let reservation = ledger.reserve_delta(&UsageDelta::new()).unwrap();
    assert_eq!(reservation.reservation_id(), format!("reservation-{huge}"));
}

#[test]
fn exported_state_round_trips_through_import() {
    let ledger = BudgetLedger::new(
        RunBudget::new()
            .with_max_tool_calls(Counter::from(10u32))
            .with_max_total_tokens(Counter::parse("1000000000000000000000").unwrap())
            .try_with_max_cost_str("5.00")
            .unwrap()
            .try_with_max_duration_seconds(2.5)
            .unwrap(),
    );
    ledger
        .record_model_call(
            ModelCallCharge::new()
                .tokens(Counter::parse("100000000000000000000").unwrap())
                .cost_str("0.10")
                .unwrap(),
        )
        .unwrap();
    ledger
        .record_tool_call(
            ToolCallCharge::new()
                .duration_seconds(0.0000005)
                .artifact_bytes(64u32)
                .cost_str("0.20")
                .unwrap(),
        )
        .unwrap();
    let held = ledger
        .reserve_delta(&UsageDelta::new().tool_calls(2u32).cost_str("0.25").unwrap())
        .unwrap();
    let state = ledger.export_state();

    let restored = BudgetLedger::default();
    restored.import_state(&state).unwrap();
    common::assert_json_eq(&restored.export_state(), &state, "state round trip");
    assert_eq!(restored.usage(), ledger.usage());
    assert_eq!(restored.reserved(), ledger.reserved());
    assert_eq!(restored.reservations(), vec![held]);
}

#[test]
fn imported_usage_above_limits_clamps_available_without_underflow() {
    let ledger = BudgetLedger::default();
    ledger
        .import_state(&json!({
            "version": 1,
            "usage": {"cost": "5.00", "tool_calls": 9, "duration_seconds": 9000.0},
            "reserved": {"cost": "0", "tool_calls": 0}
        }))
        .unwrap();
    let available = ledger.available().unwrap();
    assert_eq!(available.cost().to_python_string(), "0");
    assert_eq!(available.tool_calls(), &Counter::from(55u32));
    assert_eq!(available.duration_seconds(), 0.0);
    assert!(ledger.is_exhausted(None).unwrap());
}

#[test]
fn concurrent_charges_are_serialized_without_lost_updates() {
    let ledger = Arc::new(BudgetLedger::new(
        RunBudget::new().with_max_tool_calls(Counter::from(10_000u32)),
    ));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let ledger = Arc::clone(&ledger);
        handles.push(thread::spawn(move || {
            for _ in 0..100 {
                ledger
                    .consume_delta(&UsageDelta::new().tool_calls(1u32))
                    .unwrap();
            }
        }));
    }
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(ledger.usage().tool_calls(), &Counter::from(800u32));
}

#[test]
fn concurrent_reservations_get_unique_ids_and_release_cleanly() {
    let ledger = Arc::new(BudgetLedger::new(
        RunBudget::new().with_max_tool_calls(Counter::from(1_000_000u32)),
    ));
    let mut handles = Vec::new();
    for _ in 0..8 {
        let ledger = Arc::clone(&ledger);
        handles.push(thread::spawn(move || {
            let mut ids = Vec::new();
            for _ in 0..25 {
                let reservation = ledger
                    .reserve_delta(&UsageDelta::new().tool_calls(1u32))
                    .unwrap();
                ids.push(reservation.reservation_id().to_owned());
                ledger.release(&reservation).unwrap();
            }
            ids
        }));
    }
    let mut ids = Vec::new();
    for handle in handles {
        ids.extend(handle.join().unwrap());
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 200, "reservation ids must never repeat");
    assert_eq!(ledger.usage(), BudgetUsage::new());
    assert_eq!(ledger.reserved(), BudgetUsage::new());
    assert!(ledger.reservations().is_empty());
}

#[test]
fn can_consume_matches_check_without_mutating() {
    let ledger = ledger_with(1, 10);
    assert!(
        ledger
            .can_consume_delta(&UsageDelta::new().tool_calls(1u32))
            .unwrap()
    );
    assert!(
        !ledger
            .can_consume_delta(&UsageDelta::new().tool_calls(2u32))
            .unwrap()
    );
    assert_eq!(ledger.usage(), BudgetUsage::new());
    assert!(
        !ledger
            .can_consume_delta(&UsageDelta::new().duration_seconds(f64::NAN))
            .unwrap()
    );
}

#[test]
fn is_exhausted_rejects_unknown_dimension_names() {
    let ledger = ledger_with(1, 10);
    let error = ledger.is_exhausted_named("nope").unwrap_err();
    assert_eq!(error.contract_error_name(), "ValueError");
    assert_eq!(error.to_string(), "'nope' is not a valid BudgetDimension");
}

#[test]
fn state_import_duration_accepts_json_ints_and_rejects_float_overflow() {
    let ledger = BudgetLedger::default();
    ledger
        .import_state(&json!({
            "version": 1,
            "usage": {"duration_seconds": 2},
            "reserved": {}
        }))
        .unwrap();
    assert_eq!(ledger.usage().duration_seconds(), 2.0);

    let huge: Value = serde_json::from_str(&format!("1{}", "0".repeat(400))).unwrap();
    let error = ledger
        .import_state(&json!({
            "version": 1,
            "usage": {"duration_seconds": huge},
            "reserved": {}
        }))
        .unwrap_err();
    assert_eq!(error.contract_error_name(), "BudgetStateError");
    assert_eq!(
        ledger.usage().duration_seconds(),
        2.0,
        "import stayed atomic"
    );
}

#[test]
fn state_import_accepts_unicode_numeric_text_and_rejects_lookalikes() {
    let ledger = BudgetLedger::default();
    ledger
        .import_state(&json!({
            "version": 1,
            "budget": {"max_cost": "\u{0669}.\u{0669}"},
            "usage": {
                "cost": "\u{0661}.\u{0662}\u{0663}",
                "duration_seconds": "\u{0661}\u{0662}.\u{0662}\u{0665}"
            },
            "reserved": {}
        }))
        .unwrap();
    assert_eq!(ledger.budget().max_cost().to_python_string(), "9.9");
    assert_eq!(ledger.usage().cost().to_python_string(), "1.23");
    assert_eq!(ledger.usage().duration_seconds(), 12.25);

    // `Decimal`-only whitespace separators work through the state path too.
    ledger
        .import_state(&json!({
            "version": 1,
            "usage": {"cost": "\u{1C}1\u{1C}"},
            "reserved": {}
        }))
        .unwrap();
    assert_eq!(ledger.usage().cost().to_python_string(), "1");

    // Non-`Nd` numeric lookalikes and out-of-range construction exponents are
    // rejected transactionally.
    let before = ledger.export_state();
    for invalid in [
        json!({"version": 1, "usage": {"cost": "\u{00B2}"}, "reserved": {}}),
        json!({"version": 1, "usage": {"cost": "\u{00BD}"}, "reserved": {}}),
        json!({"version": 1, "usage": {"cost": "\u{2160}"}, "reserved": {}}),
        json!({"version": 1, "usage": {"cost": "1e-1999999999999999998"}, "reserved": {}}),
        json!({"version": 1, "usage": {"cost": "1e1000000000000000000"}, "reserved": {}}),
        json!({"version": 1, "usage": {"duration_seconds": "\u{1C}1\u{1C}"}, "reserved": {}}),
    ] {
        assert!(ledger.import_state(&invalid).is_err(), "{invalid}");
        common::assert_json_eq(&ledger.export_state(), &before, "transactional import");
    }

    // The exact construction boundaries stay accepted on every entry path.
    ledger
        .import_state(&json!({
            "version": 1,
            "budget": {"max_cost": "1e-1999999999999999997"},
            "usage": {"cost": "99e999999999999999998"},
            "reserved": {}
        }))
        .unwrap();
    assert_eq!(
        ledger.budget().max_cost().to_python_string(),
        "1E-1999999999999999997"
    );
    assert_eq!(
        ledger.usage().cost().to_python_string(),
        "9.9E+999999999999999999"
    );
}
