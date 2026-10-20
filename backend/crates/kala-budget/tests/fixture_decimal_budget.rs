//! Gate over every case in `backend/fixtures/corpus/v1/decimal_budget.json`.
//!
//! The fixture is the frozen Python oracle output for the decimal/budget
//! corpus. This test consumes all 85 cases without skipping or weakening a
//! case: successful values are compared exactly, and rejected cases must be
//! rejected by the native types with the matching failure category.

use kala_budget::decimal::{Decimal, DecimalParseError};
use kala_budget::{
    BudgetError, BudgetLedger, BudgetUsage, Counter, ModelCallCharge, RunBudget, ToolCallCharge,
    UsageDelta, clamp_unit_interval,
};
use serde_json::{Value, json};
use std::path::PathBuf;

mod common;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/v1/decimal_budget.json")
}

fn fixture() -> Value {
    let text = std::fs::read_to_string(fixture_path()).expect("fixture is readable");
    serde_json::from_str(&text).expect("fixture is valid JSON")
}

fn decimal(text: &str) -> Decimal {
    Decimal::parse(text).expect("fixture decimal parses")
}

fn success<'a>(entry: &'a Value, id: &str) -> &'a Value {
    let expected = &entry["expected"];
    assert_eq!(expected["ok"], Value::Bool(true), "{id} oracle failed");
    &expected["value"]
}

#[test]
fn consumes_all_85_cases() {
    let data = fixture();
    assert_eq!(data["kind"], "decimal_budget");
    let mut consumed = 0usize;
    consumed += decode_cases(&data);
    consumed += run_budget_cases(&data);
    consumed += usage_cases(&data);
    consumed += duration_cases(&data);
    consumed += ledger_cases(&data);
    consumed += clamp_cases(&data);
    assert_eq!(
        consumed, 85,
        "the whole decimal/budget corpus must be gated"
    );
}

fn decode_cases(data: &Value) -> usize {
    let cases = data["decode_cases"].as_array().expect("decode array");
    assert_eq!(cases.len(), 17);
    assert_eq!(
        data["decimal_strings"].as_array().map(Vec::len),
        Some(cases.len())
    );
    for entry in cases {
        let id = entry["id"].as_str().expect("id");
        let input = entry["input"].as_str().expect("input");
        let expected = &entry["expected"];
        let value = Decimal::parse(input).unwrap_or_else(|error| panic!("{id}: {error}"));
        assert_eq!(
            i64::from(value.is_negative()),
            expected["sign"].as_i64().expect("sign"),
            "{id}"
        );
        let digits: Vec<i64> = value.as_tuple().1.into_iter().map(i64::from).collect();
        assert_eq!(
            digits,
            expected["digits"]
                .as_array()
                .expect("digits")
                .iter()
                .map(|digit| digit.as_i64().expect("digit"))
                .collect::<Vec<_>>(),
            "{id}"
        );
        assert_eq!(
            value.exponent(),
            expected["exponent"].as_i64().expect("exponent"),
            "{id}"
        );
        assert_eq!(
            value.to_python_string(),
            expected["str"].as_str().expect("str"),
            "{id}"
        );
    }
    cases.len()
}

fn run_budget_cases(data: &Value) -> usize {
    let cases = data["run_budget_cases"]
        .as_array()
        .expect("run budget array");
    assert_eq!(cases.len(), 23);
    for entry in cases {
        let id = entry["id"].as_str().expect("id");
        let max_cost = entry["max_cost"].as_str().expect("max_cost");
        let outcome = RunBudget::new().try_with_max_cost_str(max_cost);
        if entry["expected"]["ok"] == Value::Bool(true) {
            let budget = outcome.unwrap_or_else(|error| panic!("{id}: {error}"));
            common::assert_json_eq(
                &budget.as_dict(),
                &entry["expected"]["value"],
                &format!("{id} accepted value"),
            );
        } else {
            let error = outcome.expect_err(&format!("{id} must be rejected"));
            let category = match &error {
                BudgetError::Validation(_) => "negative",
                BudgetError::DecimalParse(error) => match error {
                    DecimalParseError::NonFinite => "nonfinite",
                    other => panic!("{id}: unexpected decimal parse error {other}"),
                },
                other => panic!("{id}: unexpected error {other}"),
            };
            let expected_category = match Decimal::parse(max_cost) {
                Err(DecimalParseError::NonFinite) => "nonfinite",
                Err(other) => panic!("{id}: unexpected oracle literal error {other}"),
                Ok(value) if value.is_negative() && !value.is_zero() => "negative",
                Ok(_) => panic!("{id}: oracle rejected a valid value"),
            };
            assert_eq!(category, expected_category, "{id}");
        }
    }
    cases.len()
}

fn usage_cases(data: &Value) -> usize {
    let cases = data["usage_cases"].as_array().expect("usage array");
    assert_eq!(cases.len(), 20);
    for entry in cases {
        let id = entry["id"].as_str().expect("id");
        if entry.get("left").is_some() {
            let left = BudgetUsage::new()
                .try_with_cost_str(entry["left"].as_str().expect("left"))
                .unwrap();
            let right = BudgetUsage::new()
                .try_with_cost_str(entry["right"].as_str().expect("right"))
                .unwrap();
            let sum = left
                .add(&right)
                .unwrap_or_else(|error| panic!("{id}: {error}"));
            common::assert_json_eq(&sum.as_dict(), success(entry, id), id);
        } else {
            let usage = BudgetUsage::new()
                .try_with_cost_str(entry["cost"].as_str().expect("cost"))
                .unwrap();
            common::assert_json_eq(&usage.as_dict(), &entry["expected"], id);
        }
    }
    cases.len()
}

fn duration_cases(data: &Value) -> usize {
    let cases = data["duration_cases"].as_array().expect("duration array");
    assert_eq!(cases.len(), 13);
    for entry in cases {
        let id = entry["id"].as_str().expect("id");
        let expected = success(entry, id);
        let usage = if let Some(pair) = entry["seconds"].as_array() {
            let left = BudgetUsage::new()
                .try_with_duration_seconds(pair[0].as_f64().expect("left seconds"))
                .unwrap();
            let right = BudgetUsage::new()
                .try_with_duration_seconds(pair[1].as_f64().expect("right seconds"))
                .unwrap();
            left.add(&right).unwrap()
        } else {
            BudgetUsage::new()
                .try_with_duration_seconds(entry["seconds"].as_f64().expect("seconds"))
                .unwrap()
        };
        common::assert_json_eq(&usage.as_dict(), expected, id);
    }
    cases.len()
}

fn ledger_cases(data: &Value) -> usize {
    let cases = data["ledger_cases"].as_array().expect("ledger array");
    assert_eq!(cases.len(), 1);
    for entry in cases {
        let expected = success(entry, "ledger.scenario");
        let observations = expected["observations"].as_array().expect("observations");
        let mut produced: Vec<Value> = Vec::new();

        let ledger = BudgetLedger::new(budget());
        produced.push(json!({"step": "initial", "state": ledger.export_state()}));

        let usage = ledger
            .record_model_call(
                ModelCallCharge::new()
                    .tokens(100u32)
                    .cost_str("0.10")
                    .unwrap(),
            )
            .unwrap();
        produced.push(json!({"step": "model_call", "usage": usage.as_dict()}));

        let usage = ledger
            .record_tool_call(
                ToolCallCharge::new()
                    .duration_seconds(0.0000005)
                    .artifact_bytes(64u32)
                    .cost_str("0.20")
                    .unwrap(),
            )
            .unwrap();
        produced.push(json!({"step": "tool_call", "usage": usage.as_dict()}));

        let reservation = ledger
            .reserve_delta(
                &UsageDelta::new()
                    .total_tokens(200u32)
                    .cost_str("0.25")
                    .unwrap(),
            )
            .unwrap();
        produced.push(json!({
            "step": "reserve",
            "reservation": reservation.as_dict(),
            "state": ledger.export_state()
        }));

        let usage = ledger
            .commit_delta(
                &reservation,
                &UsageDelta::new()
                    .total_tokens(150u32)
                    .cost_str("0.10")
                    .unwrap(),
            )
            .unwrap();
        produced.push(json!({"step": "commit_reservation", "usage": usage.as_dict()}));
        produced.push(json!({"step": "final", "state": ledger.export_state()}));

        let imported = BudgetLedger::new(RunBudget::new());
        imported.import_state(&ledger.export_state()).unwrap();
        produced.push(json!({
            "step": "import_round_trip",
            "state": imported.export_state(),
            "equal_to_export": imported.export_state() == ledger.export_state()
        }));

        let error = ledger
            .record_cost(decimal("1.0"))
            .expect_err("cost exhaustion");
        produced.push(json!({
            "step": "exhaust_cost",
            "expected": {
                "ok": false,
                "error_type": error.contract_error_name(),
                "message": error.to_string()
            }
        }));

        common::assert_json_eq(
            &Value::Array(produced),
            &Value::Array(observations.clone()),
            "ledger observation sequence",
        );
    }
    cases.len()
}

fn budget() -> RunBudget {
    RunBudget::new()
        .with_max_model_calls(Counter::from(4u32))
        .with_max_tool_calls(Counter::from(5u32))
        .with_max_total_tokens(Counter::from(1_000u32))
        .with_max_artifact_bytes(Counter::from(10_000u32))
        .try_with_max_duration_seconds(10.0)
        .unwrap()
        .try_with_max_cost_str("1.00")
        .unwrap()
        .with_max_replanning_attempts(Counter::from(2u32))
}

fn clamp_cases(data: &Value) -> usize {
    let cases = data["clamp_cases"].as_array().expect("clamp array");
    assert_eq!(cases.len(), 11);
    for entry in cases {
        let id = entry["id"].as_str().expect("id");
        let input = &entry["input"];
        let value = if let Some(literal) = input.get("float_literal") {
            match literal.as_str().expect("literal") {
                "inf" => f64::INFINITY,
                "nan" => f64::NAN,
                other => panic!("{id}: unexpected float literal {other}"),
            }
        } else {
            input["value"].as_f64().expect("value")
        };
        let outcome = clamp_unit_interval(value);
        if entry["expected"]["ok"] == Value::Bool(true) {
            let clamped = outcome.unwrap_or_else(|error| panic!("{id}: {error}"));
            assert_eq!(
                clamped,
                entry["expected"]["value"].as_f64().unwrap(),
                "{id}"
            );
        } else {
            assert!(outcome.is_err(), "{id} must be rejected");
        }
    }
    cases.len()
}
