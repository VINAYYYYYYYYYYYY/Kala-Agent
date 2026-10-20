//! Live-Python deterministic differential tests.
//!
//! These tests execute a seeded, reproducible workload against both the real
//! CPython `decimal`/`kala_agent.kernel.budget` implementation and the native
//! Rust crate, then compare the results structurally. Python is a test oracle
//! only; the crate never invokes it at runtime.
//!
//! The oracle requires the repository virtual environment
//! (`<repo>/.venv/bin/python`, overridable through `KALA_PYTHON`). A missing
//! oracle fails the test loudly instead of silently skipping the gate.

use kala_budget::{
    BudgetError, BudgetLedger, BudgetReservation, BudgetUsage, Counter, Decimal, ModelCallCharge,
    RunBudget, ToolCallCharge, UsageDelta,
};
use serde_json::{Value, json};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::str::FromStr;

mod common;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    manifest_dir()
        .join("../../..")
        .canonicalize()
        .expect("repository root")
}

fn python_executable() -> PathBuf {
    if let Ok(path) = std::env::var("KALA_PYTHON") {
        return PathBuf::from(path);
    }
    let venv = repo_root().join(".venv/bin/python");
    assert!(
        venv.exists(),
        "live Python oracle requires {} (set KALA_PYTHON to override)",
        venv.display()
    );
    venv
}

fn run_python(script: &str, input: &Value, with_kala_agent: bool) -> Value {
    let script_path = manifest_dir().join("tests").join(script);
    let mut command = Command::new(python_executable());
    if with_kala_agent {
        command.env("KALA_SRC", repo_root().join("src"));
    }
    let mut child = command
        .arg(&script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("failed to spawn {}: {error}", script_path.display()));
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(input.to_string().as_bytes())
        .expect("write oracle input");
    let output = child.wait_with_output().expect("wait for oracle");
    assert!(
        output.status.success(),
        "oracle {script} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "oracle {script} did not return JSON: {error}\nstdout: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// Deterministic linear congruential generator for reproducible workloads.
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    fn range(&mut self, low: i64, high: i64) -> i64 {
        let span = (high - low + 1) as u64;
        low + (self.next() % span) as i64
    }

    fn digit(&mut self) -> u8 {
        (self.next() % 10) as u8
    }
}

fn random_decimal_text(rng: &mut Lcg) -> String {
    let negative = rng.next() % 3 == 0;
    let integer_len = rng.range(1, 32) as usize;
    let fraction_len = if rng.next() % 2 == 0 {
        rng.range(0, 32) as usize
    } else {
        0
    };
    let mut text = String::new();
    if negative {
        text.push('-');
    }
    for _ in 0..integer_len {
        text.push(char::from(b'0' + rng.digit()));
    }
    if fraction_len > 0 {
        text.push('.');
        for _ in 0..fraction_len {
            text.push(char::from(b'0' + rng.digit()));
        }
    }
    if rng.next() % 2 == 0 {
        let magnitude = match rng.next() % 8 {
            0 => rng.range(0, 40),
            1 => rng.range(0, 1_000),
            2 => rng.range(0, 1_000_000),
            3 => rng.range(0, 999_999_999_999_999_999),
            _ => rng.range(0, 100),
        };
        let exponent = if rng.next() % 2 == 0 {
            magnitude
        } else {
            -magnitude
        };
        text.push('e');
        text.push_str(&exponent.to_string());
    }
    text
}

/// A deterministic literal whose *stored* exponent (written exponent minus the
/// fraction length) lands within a few steps of `MIN_ETINY`/`MAX_EMAX`, with a
/// random coefficient and fraction split. Repair round 2 uses these to compare
/// CPython's construction bounds and the resulting arithmetic exhaustively
/// around the boundary.
fn boundary_decimal_text(rng: &mut Lcg) -> String {
    const MAX_EMAX: i128 = 999_999_999_999_999_999;
    const MIN_ETINY: i128 = -1_999_999_999_999_999_997;

    let length = rng.range(1, 40) as usize;
    let mut digits = String::with_capacity(length);
    for _ in 0..length {
        digits.push(char::from(b'0' + rng.digit()));
    }
    let point = rng.range(1, length as i64) as usize; // integer part always nonempty
    let fraction_len = length - point;
    let stored = if rng.next() % 2 == 0 {
        MAX_EMAX
    } else {
        MIN_ETINY
    } + i128::from(rng.range(-4, 4));
    let written = stored + fraction_len as i128;

    let mut text = String::with_capacity(length + 24);
    if rng.next() % 4 == 0 {
        text.push('-');
    }
    text.push_str(&digits[..point]);
    if fraction_len > 0 {
        text.push('.');
        text.push_str(&digits[point..]);
    }
    text.push('e');
    if written >= 0 {
        text.push('+');
    }
    text.push_str(&written.to_string());
    text
}

/// A digit rendered in ASCII or one of the Unicode `Nd` blocks used by the
/// review probes (Arabic-Indic, fullwidth, mathematical double-struck).
fn unicode_digit(rng: &mut Lcg) -> char {
    let value = rng.next() % 10;
    let digit = || u32::try_from(value).expect("digit");
    match rng.next() % 4 {
        0 => char::from(b'0' + u8::try_from(value).expect("digit")),
        1 => char::from_u32(0x0660 + digit()).expect("Arabic-Indic digit"),
        2 => char::from_u32(0xFF10 + digit()).expect("fullwidth digit"),
        _ => char::from_u32(0x1D7CE + digit()).expect("mathematical digit"),
    }
}

/// A deterministic Decimal literal mixing Unicode `Nd` digits, underscores,
/// `Decimal`-only whitespace, exponents and non-`Nd` lookalikes. The live
/// oracle decides acceptance; the point is that native parsing agrees.
fn random_unicode_decimal_text(rng: &mut Lcg) -> String {
    let mut text = String::new();
    if rng.next() % 3 == 0 {
        text.push(if rng.next() % 2 == 0 { '-' } else { '+' });
    }
    if rng.next() % 6 == 0 {
        text.push('_');
    }
    let integer_len = rng.range(1, 6);
    for index in 0..integer_len {
        if index > 0 && rng.next() % 5 == 0 {
            text.push('_');
        }
        text.push(unicode_digit(rng));
    }
    if rng.next() % 2 == 0 {
        text.push('.');
        let fraction_len = rng.range(0, 4);
        for index in 0..fraction_len {
            if index > 0 && rng.next() % 5 == 0 {
                text.push('_');
            }
            text.push(unicode_digit(rng));
        }
    }
    if rng.next() % 2 == 0 {
        text.push(if rng.next() % 2 == 0 { 'e' } else { 'E' });
        if rng.next() % 2 == 0 {
            text.push(if rng.next() % 2 == 0 { '-' } else { '+' });
        }
        let exponent_len = rng.range(1, 3);
        for index in 0..exponent_len {
            if index > 0 && rng.next() % 5 == 0 {
                text.push('_');
            }
            text.push(unicode_digit(rng));
        }
    }
    if rng.next() % 8 == 0 {
        let lookalike = ['\u{00B2}', '\u{00BD}', '\u{2160}', 'x'][(rng.next() % 4) as usize];
        let target = rng.range(0, text.chars().count() as i64) as usize;
        let byte_index = text
            .char_indices()
            .nth(target)
            .map_or(text.len(), |(index, _)| index);
        text.insert(byte_index, lookalike);
    }
    if rng.next() % 3 == 0 {
        let space = [
            ' ', '\u{1C}', '\u{1D}', '\u{3000}', '\u{85}', '\u{A0}', '\t',
        ][(rng.next() % 7) as usize];
        match rng.next() % 3 {
            0 => text.insert(0, space),
            1 => text.push(space),
            _ => {
                text.insert(0, space);
                text.push(space);
            }
        }
    }
    text
}

fn describe_ours(text: &str) -> Value {
    match Decimal::parse(text) {
        Ok(value) => {
            let (sign, digits, exponent) = value.as_tuple();
            json!({
                "ok": true,
                "str": value.to_python_string(),
                "tuple": [sign, digits, exponent],
            })
        }
        Err(error) => {
            let kind = match error {
                kala_budget::DecimalParseError::NonFinite => "NonFinite",
                _ => "InvalidOperation",
            };
            json!({"ok": false, "error": kind})
        }
    }
}

fn operate_ours(left: &str, right: &str, subtract: bool) -> Value {
    let (Ok(left), Ok(right)) = (Decimal::parse(left), Decimal::parse(right)) else {
        return json!({"ok": false, "error": "PreviousError"});
    };
    let result = if subtract {
        left.sub(&right)
    } else {
        left.add(&right)
    };
    match result {
        Ok(value) => {
            let (sign, digits, exponent) = value.as_tuple();
            json!({
                "ok": true,
                "str": value.to_python_string(),
                "tuple": [sign, digits, exponent],
            })
        }
        Err(kala_budget::DecimalError::Overflow) => json!({"ok": false, "error": "Overflow"}),
        Err(_) => json!({"ok": false, "error": "InvalidOperation"}),
    }
}

#[test]
fn decimal_parse_and_arithmetic_match_live_python() {
    let mut rng = Lcg::new(0x5EED_2026_1019);
    let mut cases: Vec<(String, String)> = Vec::new();
    let fixed = [
        ("0", "0.0"),
        ("-0", "-0.0"),
        ("0E-1000000", "1"),
        ("1.00", "2.00"),
        ("0.10", "0.2"),
        ("0.1", "0.2"),
        ("1e+40", "1e-30"),
        ("1e+999999", "1e-999999"),
        ("1e-999999", "1e-999999"),
        ("1.2345678901234567890123456785", "1e-40"),
        ("123456789012345678901234567890.12345", "0.00001"),
        ("9e999999", "9e999999"),
        ("1e+999999", "-1e+999999"),
        ("-1.00", "1.00"),
        ("5.0000000000000000000000000005", "-5"),
        ("0.000001", "0.0000005"),
        ("1_000.0_1", " 2 "),
        ("1e-1000026", "1e-1000026"),
        ("1e-1000027", "0"),
        ("1e999999999", "-1e999999999"),
        ("1e-999999999", "1e-999999999"),
        // Repair round 2: Unicode Nd digits, `Decimal`-only whitespace and
        // CPython construction exponent boundaries (live-oracle parity).
        ("\u{0661}.\u{0662}\u{0663}", "0"),
        ("-\u{0661}.\u{0662}\u{0663}", "0"),
        ("+\u{0661}", "0"),
        ("\u{FF11}\u{FF12}.\u{FF13}\u{FF14}", "1"),
        ("\u{1D7D9}\u{1D7DA}.\u{1D7DB}", "2"),
        ("1e\u{0662}", "1"),
        ("1e+\u{0662}", "1e-\u{0662}"),
        ("\u{0661}e\u{FF12}", "0"),
        ("\u{2212}1", "1"),
        ("\u{1C}1\u{1C}", "1"),
        ("\u{1C}1", "1"),
        ("1\u{1F}", "0"),
        ("\u{85}1\u{85}", "1"),
        ("\u{A0}1\u{3000}", "1"),
        ("\u{00B2}", "1"),
        ("\u{00BD}", "1"),
        ("\u{2160}", "1"),
        ("\u{180E}1\u{180E}", "1"),
        ("1e-1999999999999999997", "1"),
        ("1e-1999999999999999998", "1"),
        ("0e-1999999999999999997", "0"),
        ("1e-1999999999999999997", "1e-1999999999999999997"),
        ("1e-1999999999999999997", "1"),
        ("1e-1999999999999999997", "-1e-1999999999999999996"),
        ("1e999999999999999999", "1"),
        ("1e999999999999999999", "-1e999999999999999999"),
        ("1e999999999999999999", "1e-1999999999999999997"),
        ("99e999999999999999998", "0"),
        ("99e999999999999999999", "1"),
        ("0.1e1000000000000000000", "1"),
        ("1.0e-1999999999999999996", "1"),
        ("1.0e-1999999999999999997", "1"),
        ("0e1000000000000000000", "1"),
        ("1e000000000000000000000000000000001", "1"),
        ("123456789012345678901234567890e999999999999999969", "1"),
        ("123456789012345678901234567890e999999999999999971", "1"),
        ("7.0000000000000000000000000005", "0"),
        ("-0.000", "0E+5"),
        ("4.9999999999999999999999999995", "1e-30"),
        ("", "1"),
        ("1e", "1"),
        ("1.2.3", "1"),
        ("1e+", "1"),
        ("0x10", "1"),
        ("- 1", "1"),
        ("1e1000000000000000000", "1"),
        ("1e-10000000000000000000", "1"),
        ("1_000", "0.5"),
        ("  1.5  ", "0.25"),
        (".5", "5."),
    ];
    for (left, right) in fixed {
        cases.push((left.to_owned(), right.to_owned()));
    }
    for _ in 0..1_000 {
        cases.push((random_decimal_text(&mut rng), random_decimal_text(&mut rng)));
    }
    // Construction-boundary stress: stored exponents within a few steps of
    // `MIN_ETINY`/`MAX_EMAX`, random coefficients and fraction shifts.
    for _ in 0..400 {
        cases.push((
            boundary_decimal_text(&mut rng),
            boundary_decimal_text(&mut rng),
        ));
    }
    // Mixed Unicode-digit/underscore/whitespace literals, accepted or rejected
    // exactly as live CPython decides.
    for _ in 0..200 {
        cases.push((
            random_unicode_decimal_text(&mut rng),
            random_unicode_decimal_text(&mut rng),
        ));
    }
    // Large coefficients (arbitrary precision) at both construction bounds.
    let wide = "1".repeat(1_000);
    cases.push((format!("{wide}e-1999999999999999997"), "0".to_owned()));
    cases.push((format!("{wide}e-1999999999999999997"), "1".to_owned()));
    cases.push((format!("{wide}e-1999999999999999998"), "1".to_owned()));
    cases.push((
        format!("{wide}e999999999999999000"),
        "1e-1999999999999999997".to_owned(),
    ));
    cases.push((format!("{wide}e999999999999999001"), "1".to_owned()));

    let oracle_input = json!({
        "decimal_cases": cases
            .iter()
            .map(|(left, right)| json!({"left": left, "right": right}))
            .collect::<Vec<_>>(),
        "duration_cases": [],
        "duration_sums": [],
    });
    let oracle = run_python("oracle_decimal.py", &oracle_input, false);
    let oracle_rows = oracle["decimal"].as_array().expect("decimal rows");
    assert_eq!(oracle_rows.len(), cases.len());

    for (index, (left, right)) in cases.iter().enumerate() {
        let row = &oracle_rows[index];
        common::assert_json_eq(&describe_ours(left), &row["left"], &format!("parse {left}"));
        common::assert_json_eq(
            &describe_ours(right),
            &row["right"],
            &format!("parse {right}"),
        );
        common::assert_json_eq(
            &operate_ours(left, right, false),
            &row["add"],
            &format!("{left} + {right}"),
        );
        common::assert_json_eq(
            &operate_ours(left, right, true),
            &row["sub"],
            &format!("{left} - {right}"),
        );
    }
}

#[test]
fn duration_grid_matches_live_python() {
    let mut rng = Lcg::new(0x0D19_2026_5EED);
    let mut cases: Vec<f64> = vec![
        0.0,
        -0.0,
        1e-7,
        5e-7,
        1.5e-6,
        2.5e-6,
        0.0000005,
        1.2345675,
        1.2345685,
        123.4567895,
        3600.0,
        0.1,
        0.2,
        0.3,
        1e300,
        1e-300,
    ];
    for _ in 0..500 {
        let choice = rng.next() % 4;
        let value = match choice {
            0 => rng.range(0, 1_000_000) as f64 / 1_000_000.0,
            1 => rng.range(0, 10_000_000) as f64 / 1_000_000.0,
            2 => (rng.range(0, 1_000_000_000_000) as f64) / 1_000_000.0,
            _ => rng.range(0, 1_000_000_000_000_000_000) as f64 / 1_000_000.0,
        };
        cases.push(value);
    }
    let mut sums: Vec<(f64, f64)> = vec![
        (0.0000005, 0.0000005),
        (0.1, 0.2),
        (1.234567, 2.345679),
        (123.456789, 0.000001),
    ];
    for _ in 0..150 {
        sums.push((
            cases[rng.range(0, cases.len() as i64 - 1) as usize],
            cases[rng.range(0, cases.len() as i64 - 1) as usize],
        ));
    }

    let oracle_input = json!({
        "decimal_cases": [],
        "duration_cases": cases.iter().map(|value| json!({"seconds": value})).collect::<Vec<_>>(),
        "duration_sums": sums.iter().map(|(left, right)| json!([left, right])).collect::<Vec<_>>(),
    });
    let oracle = run_python("oracle_decimal.py", &oracle_input, false);

    let duration_rows = oracle["duration"].as_array().expect("duration rows");
    assert_eq!(duration_rows.len(), cases.len());
    for (index, seconds) in cases.iter().enumerate() {
        let row = &duration_rows[index];
        match kala_budget::normalize_duration(*seconds) {
            Ok(value) => {
                assert_eq!(row["ok"], Value::Bool(true), "case {index} {seconds}");
                common::assert_json_eq(
                    &json!(value),
                    &row["seconds"],
                    &format!("duration {seconds}"),
                );
            }
            Err(error) => {
                assert_eq!(row["ok"], Value::Bool(false), "case {index} {seconds}");
                assert_eq!(
                    error.contract_error_name(),
                    row["error"].as_str().expect("oracle error"),
                    "case {index} {seconds}"
                );
            }
        }
    }

    let sum_rows = oracle["duration_sums"].as_array().expect("duration sums");
    assert_eq!(sum_rows.len(), sums.len());
    for (index, (left, right)) in sums.iter().enumerate() {
        let left = BudgetUsage::new().try_with_duration_seconds(*left).unwrap();
        let right = BudgetUsage::new()
            .try_with_duration_seconds(*right)
            .unwrap();
        let sum = left.add(&right).expect("duration sum");
        common::assert_json_eq(
            &json!(sum.duration_seconds()),
            &sum_rows[index]["seconds"],
            &format!("duration sum {index}"),
        );
    }
}

// ---------------------------------------------------------------------------
// Ledger command-sequence differential
// ---------------------------------------------------------------------------

fn execute_ledger_command(
    ledger: &mut Option<BudgetLedger>,
    command: &Value,
) -> Result<Value, BudgetError> {
    let name = command["cmd"].as_str().expect("command name");
    match name {
        "new" => {
            let budget = RunBudget::from_json(&command["budget"])?;
            *ledger = Some(BudgetLedger::new(budget));
            Ok(Value::Null)
        }
        "record_model_call" => {
            let target = ledger.as_ref().expect("ledger");
            let mut call = ModelCallCharge::new();
            if let Some(value) = command.get("count") {
                call = call.count(Counter::parse(&value.to_string())?);
            }
            if let Some(value) = command.get("tokens") {
                call = call.tokens(Counter::parse(&value.to_string())?);
            }
            if let Some(value) = command.get("duration_seconds") {
                call = call.duration_seconds(value.as_f64().expect("duration"));
            }
            if let Some(value) = command.get("cost") {
                call = call.cost_str(value.as_str().expect("cost text"))?;
            }
            Ok(target.record_model_call(call)?.as_dict())
        }
        "record_tool_call" => {
            let target = ledger.as_ref().expect("ledger");
            let mut call = ToolCallCharge::new();
            if let Some(value) = command.get("count") {
                call = call.count(Counter::parse(&value.to_string())?);
            }
            if let Some(value) = command.get("artifact_bytes") {
                call = call.artifact_bytes(Counter::parse(&value.to_string())?);
            }
            if let Some(value) = command.get("duration_seconds") {
                call = call.duration_seconds(value.as_f64().expect("duration"));
            }
            if let Some(value) = command.get("cost") {
                call = call.cost_str(value.as_str().expect("cost text"))?;
            }
            Ok(target.record_tool_call(call)?.as_dict())
        }
        "consume" => {
            let target = ledger.as_ref().expect("ledger");
            let delta = delta_from_command(command)?;
            Ok(target.consume_delta(&delta)?.as_dict())
        }
        "reserve" => {
            let target = ledger.as_ref().expect("ledger");
            let delta = delta_from_command(command)?;
            let reservation = target.reserve_delta(&delta)?;
            Ok(json!({
                "reservation": reservation.as_dict(),
                "usage": target.usage().as_dict(),
                "state": target.export_state(),
            }))
        }
        "commit" => {
            let target = ledger.as_ref().expect("ledger");
            let reservation = find_reservation(target, command["id"].as_str().expect("id"))?;
            let delta = delta_from_command(command)?;
            Ok(target.commit_delta(&reservation, &delta)?.as_dict())
        }
        "commit_plain" => {
            let target = ledger.as_ref().expect("ledger");
            let reservation = find_reservation(target, command["id"].as_str().expect("id"))?;
            Ok(target.commit(&reservation)?.as_dict())
        }
        "commit_wrong" => {
            let target = ledger.as_ref().expect("ledger");
            let mut usage = BudgetUsage::new();
            if let Some(cost) = command["usage"].get("cost") {
                usage = usage.try_with_cost_str(cost.as_str().expect("cost text"))?;
            }
            let wrong = BudgetReservation::new(command["id"].as_str().expect("id"), usage);
            Ok(target.commit(&wrong)?.as_dict())
        }
        "release" => {
            let target = ledger.as_ref().expect("ledger");
            let reservation = find_reservation(target, command["id"].as_str().expect("id"))?;
            Ok(target.release(&reservation)?.as_dict())
        }
        "export" => Ok(ledger.as_ref().expect("ledger").export_state()),
        "import" => {
            let target = ledger.as_ref().expect("ledger");
            target.import_state(&command["state"])?;
            Ok(target.export_state())
        }
        "available" => Ok(ledger.as_ref().expect("ledger").available()?.as_dict()),
        "usage" => Ok(ledger.as_ref().expect("ledger").usage().as_dict()),
        "reserved" => Ok(ledger.as_ref().expect("ledger").reserved().as_dict()),
        "reservations" => Ok(Value::Array(
            ledger
                .as_ref()
                .expect("ledger")
                .reservations()
                .iter()
                .map(BudgetReservation::as_dict)
                .collect(),
        )),
        "is_exhausted" => {
            let target = ledger.as_ref().expect("ledger");
            let dimension = match command.get("dimension").and_then(Value::as_str) {
                Some(name) => Some(kala_budget::BudgetDimension::from_str(name)?),
                None => None,
            };
            Ok(Value::Bool(target.is_exhausted(dimension)?))
        }
        "can_consume" => {
            let target = ledger.as_ref().expect("ledger");
            let delta = delta_from_command(command)?;
            Ok(Value::Bool(target.can_consume_delta(&delta)?))
        }
        other => panic!("unknown ledger command {other}"),
    }
}

fn delta_from_command(command: &Value) -> Result<UsageDelta, BudgetError> {
    let mut delta = UsageDelta::new();
    if let Some(value) = command.get("kwargs").and_then(Value::as_object) {
        for (key, value) in value {
            match key.as_str() {
                "model_calls" => delta.model_calls = Counter::parse(&value.to_string())?,
                "tool_calls" => delta.tool_calls = Counter::parse(&value.to_string())?,
                "total_tokens" | "tokens" => {
                    delta.total_tokens = Counter::parse(&value.to_string())?
                }
                "artifact_bytes" | "artifact_size" => {
                    delta.artifact_bytes = Counter::parse(&value.to_string())?
                }
                "replanning_attempts" | "replanning" => {
                    delta.replanning_attempts = Counter::parse(&value.to_string())?
                }
                "duration_seconds" => {
                    delta.duration_seconds = value.as_f64().expect("duration");
                }
                "cost" => {
                    delta.cost = Decimal::parse(value.as_str().expect("cost text"))?;
                }
                other => panic!("unknown delta key {other}"),
            }
        }
    }
    Ok(delta)
}

fn find_reservation(ledger: &BudgetLedger, id: &str) -> Result<BudgetReservation, BudgetError> {
    ledger
        .reservations()
        .into_iter()
        .find(|reservation| reservation.reservation_id() == id)
        .ok_or_else(|| BudgetError::ReservationUnknown { id: id.to_owned() })
}

fn ledger_commands() -> Vec<Value> {
    let mut commands = Vec::new();
    commands.push(json!({"cmd": "new", "budget": {
        "max_model_calls": 4,
        "max_tool_calls": 5,
        "max_total_tokens": 1000,
        "max_artifact_bytes": 10000,
        "max_duration_seconds": 10.0,
        "max_cost": "1.00",
        "max_replanning_attempts": 2
    }}));
    commands.push(json!({"cmd": "export"}));
    commands.push(json!({"cmd": "record_model_call", "tokens": 100, "cost": "0.10"}));
    commands.push(
        json!({"cmd": "record_model_call", "count": 2, "tokens": 50, "duration_seconds": 0.5}),
    );
    commands.push(json!({"cmd": "record_tool_call", "duration_seconds": 0.0000005, "artifact_bytes": 64, "cost": "0.20"}));
    commands
        .push(json!({"cmd": "consume", "kwargs": {"tokens": 7, "duration_seconds": 0.0000005}}));
    commands.push(json!({"cmd": "reserve", "kwargs": {"total_tokens": 200, "cost": "0.25"}}));
    commands.push(json!({"cmd": "reserve", "kwargs": {"tool_calls": 1, "duration_seconds": 0.1}}));
    commands.push(json!({"cmd": "commit", "id": "reservation-1", "kwargs": {"total_tokens": 150, "cost": "0.10"}}));
    commands.push(json!({"cmd": "release", "id": "reservation-2"}));
    commands.push(json!({"cmd": "commit_plain", "id": "reservation-1"}));
    commands.push(json!({"cmd": "release", "id": "reservation-1"}));
    commands.push(json!({"cmd": "record_tool_call", "count": 2, "cost": "0.30"}));
    commands.push(json!({"cmd": "usage"}));
    commands.push(json!({"cmd": "reserved"}));
    commands.push(json!({"cmd": "reservations"}));
    commands.push(json!({"cmd": "available"}));
    commands.push(json!({"cmd": "is_exhausted", "dimension": "cost"}));
    commands.push(json!({"cmd": "is_exhausted", "dimension": null}));
    commands.push(json!({"cmd": "can_consume", "kwargs": {"cost": "0.30"}}));
    commands.push(json!({"cmd": "consume", "kwargs": {"cost": "0.30"}}));
    commands.push(json!({"cmd": "export"}));

    // Reservation error paths.
    commands.push(json!({"cmd": "commit_wrong", "id": "reservation-9", "usage": {"cost": "0.1"}}));
    commands.push(json!({"cmd": "release", "id": "reservation-9"}));
    commands.push(json!({"cmd": "reserve", "kwargs": {"tool_calls": 3}}));
    commands
        .push(json!({"cmd": "commit_wrong", "id": "reservation-3", "usage": {"tool_calls": 1}}));
    commands.push(json!({"cmd": "commit_plain", "id": "reservation-3"}));

    // Import/round-trip and tamper behaviour. The tampered states return the
    // ledger unchanged on the Python side, so the follow-up export must match.
    commands.push(json!({"cmd": "export"}));
    commands.push(json!({"cmd": "import", "state": {"version": 99}}));
    commands.push(json!({"cmd": "import", "state": {"version": 1, "unexpected": true}}));
    commands.push(json!({"cmd": "import", "state": {"version": 1, "budget": {"extra": 1}}}));
    commands.push(json!({"cmd": "import", "state": {"version": 1, "usage": {"cost": "free"}}}));
    commands.push(json!({"cmd": "import", "state": {"version": 1, "reserved": {"cost": "0.30"}, "reservations": [{"reservation_id": "r", "usage": {"cost": "0.3"}}], "next_reservation": 4}}));
    commands.push(json!({"cmd": "export"}));
    // Numeric input shapes in imported state: JSON float costs go through
    // `Decimal(str(float))`, integers stay exact and duration floats regrid.
    commands.push(json!({"cmd": "import", "state": {"version": 1, "usage": {"cost": 0.1, "duration_seconds": 1.5, "tool_calls": 2}, "reserved": {}}}));
    commands.push(json!({"cmd": "usage"}));
    commands.push(json!({"cmd": "import", "state": {"version": 1, "usage": {"cost": 2, "duration_seconds": 0.0000005}, "reserved": {}}}));
    commands.push(json!({"cmd": "usage"}));
    commands.push(json!({"cmd": "record_model_call", "cost": "0.5"}));
    commands.push(json!({"cmd": "is_exhausted", "dimension": "model_calls"}));
    commands.push(json!({"cmd": "is_exhausted", "dimension": "cost"}));

    // Far-exponent cost arithmetic inside the ledger: a value near Emax is
    // preserved exactly, then a second add overflows the default context
    // (Python raises decimal.Overflow) and the tiny add rounds at precision
    // 28 without allocating the exponent gap.
    commands.push(json!({"cmd": "new", "budget": {"max_cost": "9.9e999999"}}));
    commands.push(json!({"cmd": "record_model_call", "cost": "9e999999"}));
    commands.push(json!({"cmd": "export"}));
    commands.push(json!({"cmd": "record_model_call", "cost": "9e999999"}));
    commands.push(json!({"cmd": "export"}));
    commands.push(json!({"cmd": "record_model_call", "cost": "1e-999999"}));
    commands.push(json!({"cmd": "usage"}));
    commands.push(json!({"cmd": "available"}));

    // JSON integer `-0` loads as Python's positive int zero; the sign must
    // not leak into the imported/exported decimal on the integer path.
    commands.push(
        serde_json::from_str(
            r#"{"cmd": "import", "state": {"version": 1, "usage": {"cost": -0}, "reserved": {}}}"#,
        )
        .expect("raw -0 import"),
    );
    commands.push(json!({"cmd": "usage"}));
    commands.push(
        serde_json::from_str(r#"{"cmd": "new", "budget": {"max_cost": -0}}"#)
            .expect("raw -0 budget"),
    );
    commands.push(json!({"cmd": "export"}));

    // A huge (but finite) duration limit overflows the microsecond grid when
    // `remaining` is materialized, exactly like Python's OverflowError.
    commands.push(json!({"cmd": "new", "budget": {"max_duration_seconds": 1e303}}));
    commands.push(json!({"cmd": "available"}));

    // Repair round 2: Unicode Nd digits and `Decimal`-only whitespace through
    // the RunBudget, charge and state-import entry paths.
    commands.push(json!({"cmd": "new", "budget": {
        "max_cost": "\u{0669}.\u{0669}",
        "max_duration_seconds": "\u{0663}\u{0666}\u{0660}\u{0660}"
    }}));
    commands.push(json!({"cmd": "export"}));
    commands.push(json!({"cmd": "record_model_call", "cost": "\u{0660}.\u{0665}"}));
    commands.push(json!({"cmd": "usage"}));
    commands.push(json!({"cmd": "import", "state": {"version": 1,
        "usage": {"cost": "\u{0661}.\u{0662}\u{0663}", "duration_seconds": "\u{0661}\u{0662}.\u{0662}\u{0665}"},
        "reserved": {}}}));
    commands.push(json!({"cmd": "usage"}));
    commands.push(json!({"cmd": "import", "state": {"version": 1,
        "usage": {"cost": "1e-1999999999999999997"}, "reserved": {}}}));
    commands.push(json!({"cmd": "usage"}));
    commands
}

#[test]
fn ledger_command_sequences_match_live_python() {
    let commands = Value::Array(ledger_commands());
    let oracle = run_python("oracle_ledger.py", &commands, true);
    let oracle_rows = oracle.as_array().expect("oracle rows");

    let mut ledger = None;
    assert_eq!(oracle_rows.len(), commands.as_array().unwrap().len());
    for (index, command) in commands.as_array().unwrap().iter().enumerate() {
        let name = &command["cmd"];
        let expected = &oracle_rows[index];
        match execute_ledger_command(&mut ledger, command) {
            Ok(value) => {
                assert_eq!(
                    expected["ok"],
                    Value::Bool(true),
                    "command {index} {name}: Python failed with {expected}"
                );
                common::assert_json_eq(
                    &value,
                    &expected["value"],
                    &format!("command {index} {name}"),
                );
            }
            Err(error) => {
                assert_eq!(
                    expected["ok"],
                    Value::Bool(false),
                    "command {index} {name}: Rust failed with {error}"
                );
                let python_type = expected["error_type"].as_str().expect("python type");
                let native_type = error.contract_error_name();
                if python_type.starts_with("Budget")
                    || python_type.starts_with("ModelCall")
                    || python_type.starts_with("ToolCall")
                    || python_type.starts_with("Token")
                    || python_type.starts_with("Artifact")
                    || python_type.starts_with("Duration")
                    || python_type.starts_with("Cost")
                    || python_type.starts_with("Replanning")
                {
                    assert_eq!(
                        python_type, native_type,
                        "command {index} {name}: error class"
                    );
                    assert_eq!(
                        expected["message"].as_str().expect("message"),
                        error.to_string(),
                        "command {index} {name}: error text"
                    );
                } else {
                    assert_eq!(
                        python_type, native_type,
                        "command {index} {name}: error class"
                    );
                }
            }
        }
    }
}
