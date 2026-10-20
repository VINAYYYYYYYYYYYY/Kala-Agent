//! Live-Python differential for Unicode numeric inputs.
//!
//! Review round 1 reproduced valid Python inputs rejected by the native parser
//! (Unicode `Nd` digits, `Decimal`-only whitespace separators) and invalid
//! extreme construction exponents accepted natively. These tests compare the
//! public entry paths against the real CPython implementation:
//!
//! - every Unicode `Nd` codepoint parses to its digit value,
//! - every numeric-but-not-decimal codepoint is rejected by both parsers,
//! - `Decimal` trims every `str.isspace()` character while `float()` trims all
//!   of them except `U+001C..U+001F`,
//! - `float()` text parity for Unicode digits/underscores/whitespace,
//! - `RunBudget` validation and JSON output for Unicode/decimal-boundary cuts.
//!
//! Python is a test oracle only (via `tests/oracle_unicode.py`); the crate never
//! invokes it at runtime. A missing oracle fails the tests loudly.

use kala_budget::{Decimal, RunBudget, python_float_parse};
use serde_json::{Value, json};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

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

fn run_python(script: &str, input: &Value) -> Value {
    let script_path = manifest_dir().join("tests").join(script);
    let mut command = Command::new(python_executable());
    command.env("KALA_SRC", repo_root().join("src"));
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

fn char_at(codepoint: u64) -> char {
    char::from_u32(u32::try_from(codepoint).expect("codepoint fits u32")).expect("scalar value")
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

    /// A digit rendered in ASCII or one of the reviewed Unicode blocks.
    fn digit(&mut self) -> char {
        let value = self.next() % 10;
        match self.next() % 5 {
            0 => char::from(b'0' + u8::try_from(value).expect("digit")),
            1 => char::from_u32(0x0660 + u32::try_from(value).expect("digit")).expect("digit"),
            2 => char::from_u32(0xFF10 + u32::try_from(value).expect("digit")).expect("digit"),
            3 => char::from_u32(0x1D7CE + u32::try_from(value).expect("digit")).expect("digit"),
            _ => char::from(b'0' + u8::try_from(value).expect("digit")),
        }
    }
}

/// A deterministic float literal mixing Unicode digits, underscores,
/// whitespace, signs, exponents and non-`Nd` lookalikes. Most samples are
/// rejected; the point is that both parsers agree either way.
fn random_float_text(rng: &mut Lcg) -> String {
    let mut text = String::new();
    if rng.next() % 4 == 0 {
        text.push(if rng.next() % 2 == 0 { '-' } else { '+' });
    }
    let integer_len = rng.range(1, 6);
    for index in 0..integer_len {
        if index > 0 && rng.next() % 5 == 0 {
            text.push('_');
        }
        text.push(rng.digit());
    }
    if rng.next() % 2 == 0 {
        text.push('.');
        let fraction_len = rng.range(0, 4);
        for index in 0..fraction_len {
            if index > 0 && rng.next() % 5 == 0 {
                text.push('_');
            }
            text.push(rng.digit());
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
            text.push(rng.digit());
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
    match rng.next() % 8 {
        0 => text.insert(0, ' '),
        1 => text.push('\u{3000}'),
        2 => text.insert(0, '\u{1C}'),
        3 => text.push('\u{85}'),
        _ => {}
    }
    text
}

#[test]
fn unicode_digit_and_whitespace_data_matches_live_python() {
    let oracle = run_python("oracle_unicode.py", &json!({"unicode_digits": true}));

    let digits = oracle["digits"].as_array().expect("digit rows").clone();
    assert!(
        digits.len() > 700,
        "expected the full Nd table, got {}",
        digits.len()
    );
    for row in &digits {
        let codepoint = row[0].as_u64().expect("codepoint");
        let digit = row[1].as_u64().expect("digit");
        assert!(digit <= 9);
        let character = char_at(codepoint);
        let text = character.to_string();
        let parsed = Decimal::parse(&text)
            .unwrap_or_else(|error| panic!("U+{codepoint:04X} should parse: {error}"));
        let (sign, digits, exponent) = parsed.as_tuple();
        assert_eq!(
            (sign, digits, exponent),
            (0, vec![u8::try_from(digit).unwrap()], 0),
            "U+{codepoint:04X}"
        );
        assert_eq!(
            parsed.to_python_string(),
            digit.to_string(),
            "U+{codepoint:04X}"
        );
        assert_eq!(
            python_float_parse(&text).expect("float accepts Nd digits"),
            digit as f64,
            "U+{codepoint:04X}"
        );
    }

    // Superscripts, fractions, Roman numerals and every other numeric-but-not-
    // decimal character must stay rejected by both parsers.
    let non_decimal = oracle["non_decimal_numeric"]
        .as_array()
        .expect("non-decimal rows");
    assert!(!non_decimal.is_empty());
    for row in non_decimal {
        let codepoint = row.as_u64().expect("codepoint");
        let text = char_at(codepoint).to_string();
        assert!(
            Decimal::parse(&text).is_err(),
            "U+{codepoint:04X} must not be a Decimal digit"
        );
        assert!(
            python_float_parse(&text).is_err(),
            "U+{codepoint:04X} must not be a float digit"
        );
    }

    // `Decimal` trims every `str.isspace()` character; `float()` trims the same
    // set except the ASCII separators U+001C..U+001F, which Rust's
    // `char::is_whitespace` also excludes.
    let spaces = oracle["spaces"].as_array().expect("space rows");
    assert_eq!(spaces.len(), 29, "expected CPython's whitespace set");
    for row in spaces {
        let codepoint = row.as_u64().expect("codepoint");
        let character = char_at(codepoint);
        let surrounded = format!("{character}1{character}");
        assert!(
            Decimal::parse(&surrounded).is_ok(),
            "Decimal should trim U+{codepoint:04X}"
        );
        if (0x1C..=0x1F).contains(&codepoint) {
            assert!(
                python_float_parse(&surrounded).is_err(),
                "float should not trim U+{codepoint:04X}"
            );
        } else {
            assert!(
                python_float_parse(&surrounded).is_ok(),
                "float should trim U+{codepoint:04X}"
            );
        }
    }
}

#[test]
fn float_parsing_matches_live_python_for_unicode_and_underscores() {
    let mut cases: Vec<String> = vec![
        // Unicode Nd digits in mantissa and exponent, all parser entry paths.
        "\u{0661}.\u{0662}\u{0663}",
        "-\u{0661}.\u{0662}\u{0663}",
        "+\u{0661}",
        "\u{FF11}\u{FF12}.\u{FF13}\u{FF14}",
        "\u{1D7D9}\u{1D7DA}.\u{1D7DB}",
        "1e\u{0662}",
        "1e+\u{0662}",
        "1e-\u{0662}",
        "\u{0661}e\u{FF12}",
        "\u{0661}_\u{0662}",
        "\u{FF11}_\u{FF12}",
        "\u{2212}1",
        // Whitespace: the Unicode set minus the ASCII separators.
        " \u{0661} ",
        "\u{85}1\u{85}",
        "\u{A0}1\u{3000}",
        "\u{1C}1\u{1C}",
        "\u{1C}1",
        "1\u{1F}",
        "\u{180E}1\u{180E}",
        "\u{200B}1\u{200B}",
        // Underscores: between digits only, identical to CPython float.
        "1_0",
        "1_0.0_0",
        "1_0.0_0e1_0",
        "1e1_0",
        "_1",
        "1_",
        "1__0",
        "1._5",
        "1_.5",
        "1.5_",
        "1e_1",
        "1e1_",
        "1e1__0",
        "1_0e1__0",
        // Grammar edges and non-decimal lookalikes.
        "1.",
        ".5",
        "1.e2",
        "5.",
        "\u{00B2}",
        "\u{00BD}",
        "\u{2160}",
        "0x10",
        "1d0",
        "1e",
        "e1",
        " 1 ",
        "1 .0",
        "inf",
        "-infinity",
        "+Inf",
        "Infinity",
        "NaN",
        "nan",
        "nan123",
        "snan",
        "-nan",
        "-0",
        "inf_",
        "_inf",
        "infinity123",
        "1e400",
        "-1e400",
        "1e-400",
        // Wrapper text parity for the shared `kala_json::format_float` path.
        "0.0",
        "-0.0",
        "1e15",
        "1e16",
        "1e-4",
        "1e-5",
        "1.5e-7",
        "5e-324",
        "1.7976931348623157e308",
        "123456789.123456789",
        "1E+3",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    // Seeded mixed-syntax samples: Unicode digits/underscores/whitespace,
    // exponents, signs and non-`Nd` lookalikes injected at random positions.
    let mut rng = Lcg::new(0x0020_2610_19B0_D637);
    for _ in 0..300 {
        cases.push(random_float_text(&mut rng));
    }
    let oracle = run_python(
        "oracle_unicode.py",
        &json!({"float_cases": cases.iter().map(String::as_str).collect::<Vec<_>>()}),
    );
    let rows = oracle["floats"].as_array().expect("float rows");
    assert_eq!(rows.len(), cases.len());

    for (index, text) in cases.iter().enumerate() {
        let row = &rows[index];
        match python_float_parse(text) {
            Ok(value) => {
                assert_eq!(row["ok"], Value::Bool(true), "case {index} {text:?}");
                assert_eq!(
                    Decimal::python_float_repr(value),
                    row["text"].as_str().expect("float text"),
                    "case {index} {text:?}"
                );
            }
            Err(_) => {
                assert_eq!(row["ok"], Value::Bool(false), "case {index} {text:?}");
            }
        }
    }
}

#[test]
fn run_budget_matches_live_python_for_unicode_and_boundary_inputs() {
    let cases = vec![
        json!({"max_cost": "\u{0661}.\u{0662}\u{0663}"}),
        json!({"max_cost": "\u{FF11}\u{FF12}.\u{FF13}\u{FF14}"}),
        json!({"max_cost": "\u{1D7D9}\u{1D7DA}.\u{1D7DB}"}),
        json!({"max_cost": "1e-1999999999999999997"}),
        json!({"max_cost": "0.1e1000000000000000000"}),
        json!({"max_cost": "99e999999999999999998"}),
        json!({"max_cost": "1e-1999999999999999998"}),
        json!({"max_cost": "99e999999999999999999"}),
        json!({"max_cost": "1e1000000000000000000"}),
        json!({"max_cost": "\u{00B2}"}),
        json!({"max_cost": "\u{00BD}"}),
        json!({"max_cost": "\u{2160}"}),
        json!({"max_duration_seconds": "\u{0661}\u{0662}.\u{0663}"}),
        json!({"max_duration_seconds": "\u{FF11}\u{FF12}\u{FF13}"}),
        json!({"max_duration_seconds": "\u{85}1\u{85}"}),
        json!({"max_duration_seconds": "\u{1C}1\u{1C}"}),
        json!({"max_duration_seconds": "1_0"}),
        json!({"max_duration_seconds": "1__0"}),
        json!({"max_model_calls": "\u{0663}"}),
        json!({"max_replanning_attempts": -1}),
    ];
    let oracle = run_python("oracle_unicode.py", &json!({"budget_cases": cases}));
    let rows = oracle["budgets"].as_array().expect("budget rows");
    assert_eq!(rows.len(), cases.len());

    for (index, case) in cases.iter().enumerate() {
        let row = &rows[index];
        match RunBudget::from_json(case) {
            Ok(budget) => {
                assert_eq!(row["ok"], Value::Bool(true), "case {index} {case}");
                common::assert_json_eq(
                    &budget.as_dict(),
                    &row["value"],
                    &format!("budget case {index} {case}"),
                );
            }
            Err(_) => {
                assert_eq!(row["ok"], Value::Bool(false), "case {index} {case}");
            }
        }
    }
}
