//! Live Python differential tests.
//!
//! These tests are the only place Python is used. They spawn the repository
//! virtual environment (or `KALA_PYTHON`) and compare against the live product
//! implementation of `kala_agent.tools.protocol`, not against copied values.
//!
//! The float mode samples 6,000 deterministic finite IEEE-754 binary64 bit
//! patterns (plus curated boundaries) and compares CPython `repr` output. The
//! structured mode compares parsing plus canonical serialization for Unicode
//! keys/strings (including lone surrogates), large integers and nested values.
//!
//! If no Python interpreter can be started the test fails rather than silently
//! skipping: this parity gate is required, not optional.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use kala_json::{Limits, canonical_dumps, format_float, parse_strict};
use serde_json::{Value, json};

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn python_executable() -> PathBuf {
    if let Ok(path) = std::env::var("KALA_PYTHON") {
        return PathBuf::from(path);
    }
    let venv = repository_root().join(".venv/bin/python");
    if venv.is_file() {
        return venv;
    }
    let python3 = Path::new("python3");
    if Command::new(python3)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
    {
        return python3.to_path_buf();
    }
    panic!(
        "no Python oracle available; set KALA_PYTHON to the repository virtual \
         environment interpreter"
    );
}

fn oracle_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/python_oracle.py")
}

fn run_oracle(mode: &str, input: &[u8]) -> Vec<u8> {
    let mut child = Command::new(python_executable())
        .arg(oracle_script())
        .arg(mode)
        .env("PYTHONPATH", repository_root().join("src"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Python oracle");
    child
        .stdin
        .as_mut()
        .expect("oracle stdin")
        .write_all(input)
        .expect("write oracle input");
    drop(child.stdin.take());
    let output = child.wait_with_output().expect("wait for oracle");
    assert!(
        output.status.success(),
        "Python oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

struct XorShift64(u64);

impl XorShift64 {
    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }
}

fn finite_bits(rng: &mut XorShift64) -> u64 {
    loop {
        let bits = rng.next();
        if bits & 0x7FF0_0000_0000_0000 != 0x7FF0_0000_0000_0000 {
            return bits;
        }
    }
}

fn curated_bits() -> Vec<u64> {
    let values = [
        0.0f64,
        -0.0,
        1.0,
        -1.0,
        1.5,
        0.1,
        0.3,
        1e-4,
        -1e-4,
        1e-5,
        -1e-5,
        1e-7,
        1e15,
        1e16,
        1.5e16,
        1.2345678901234568e16,
        f64::MIN_POSITIVE,
        f64::from_bits(1), // smallest subnormal
        f64::MAX,
        f64::MIN,
        9007199254740991.0,
        9007199254740992.0,
        1234567890123456.0,
        2.2250738585072014e-308,
    ];
    values.iter().map(|value| value.to_bits()).collect()
}

#[test]
fn floats_match_live_python_repr_at_scale() {
    let mut rng = XorShift64(0x4B41_4C41_4A53_4F4E);
    let mut bits = curated_bits();
    const RANDOM_SAMPLES: usize = 6_000;
    for _ in 0..RANDOM_SAMPLES {
        bits.push(finite_bits(&mut rng));
    }
    assert!(bits.len() >= 5_000);

    let mut input = String::with_capacity(bits.len() * 17);
    for pattern in &bits {
        input.push_str(&format!("{pattern:016x}\n"));
    }
    let output = run_oracle("floats", input.as_bytes());
    let oracle_lines: Vec<&str> = std::str::from_utf8(&output)
        .expect("oracle output is UTF-8")
        .lines()
        .collect();
    assert_eq!(oracle_lines.len(), bits.len(), "oracle line count mismatch");

    let mut parsed_roundtrips = 0usize;
    for (pattern, expected) in bits.iter().zip(oracle_lines) {
        let value = f64::from_bits(*pattern);
        let rust = format_float(value).unwrap_or_else(|error| panic!("{pattern:016x}: {error}"));
        assert_eq!(rust, expected, "float bits {pattern:016x}");

        // The emitted literal must parse back to the same canonical bytes.
        let reparsed = parse_strict(rust.as_bytes(), &Limits::default())
            .unwrap_or_else(|error| panic!("reparse {rust:?}: {error}"));
        assert_eq!(
            canonical_dumps(&reparsed, &Limits::default()).unwrap(),
            expected,
            "reparse {rust:?}"
        );
        parsed_roundtrips += 1;
    }
    assert_eq!(parsed_roundtrips, bits.len());
}

fn escape_code_point(point: u32, out: &mut String) {
    if point > 0xFFFF {
        let value = point - 0x1_0000;
        let high = 0xD800 + (value >> 10);
        let low = 0xDC00 + (value & 0x3FF);
        out.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
    } else {
        out.push_str(&format!("\\u{point:04x}"));
    }
}

fn random_code_point(rng: &mut XorShift64) -> u32 {
    match rng.next() % 8 {
        0 => (rng.next() % 0x80) as u32,         // ASCII, including controls
        1 => (0x80 + rng.next() % 0x800) as u32, // two-byte range
        2 => (0xD800 + rng.next() % 0x800) as u32, // lone/paired surrogate range
        _ => (rng.next() % 0x11_0000) as u32,    // full Unicode
    }
}

fn random_text_literal(rng: &mut XorShift64, length: usize) -> String {
    let mut out = String::from("\"");
    for _ in 0..length {
        escape_code_point(random_code_point(rng), &mut out);
    }
    out.push('"');
    out
}

fn random_big_integer(rng: &mut XorShift64) -> String {
    let digits = 1 + (rng.next() % 300) as usize;
    let mut out = String::with_capacity(digits);
    out.push(char::from(b'1' + (rng.next() % 9) as u8));
    for _ in 1..digits {
        out.push(char::from(b'0' + (rng.next() % 10) as u8));
    }
    if rng.next() & 1 == 0 {
        format!("-{out}")
    } else {
        out
    }
}

fn structured_sources() -> Vec<String> {
    let mut sources = Vec::new();
    let curated = [
        r#""\ud800""#.to_owned(),
        r#""\udfff""#.to_owned(),
        r#""\ud83d\ude00""#.to_owned(),
        r#""\ud83d\u0041""#.to_owned(),
        r#""\ud83d\ud83d\ude00""#.to_owned(),
        r#""\u0000\u001f\u007f\u0080""#.to_owned(),
        r#""quote \" slash \\ slash/ tab\t newline\n""#.to_owned(),
        r#"{"b":1,"a":2,"Z":3,"z":4}"#.to_owned(),
        r#"{"z":1,"\u00e4":2,"a":3,"\ud83d\ude00":4}"#.to_owned(),
        "[3,2.0,1,true,false,null]".to_owned(),
        "1180591620717411303424".to_owned(),
        "-1180591620717411303424".to_owned(),
        format!("1{}", "0".repeat(100)),
        r#"{"mixed":{"big":1180591620717411303424,"ratio":1e-05,"zero":-0.0,"text":"\u03ba \ud83d\ude00"}}"#
            .to_owned(),
    ];
    let mut large_object = String::from("{");
    for index in 0..2_049 {
        if index > 0 {
            large_object.push(',');
        }
        large_object.push_str(&format!("\"k{index}\":{index}"));
    }
    large_object.push('}');
    sources.push(large_object);

    sources.extend(curated);

    let mut rng = XorShift64(0x554E_4943_4F44_4553);
    for _ in 0..120 {
        let length = (rng.next() % 40) as usize;
        sources.push(random_text_literal(&mut rng, length));
    }
    for _ in 0..40 {
        let count = 1 + (rng.next() % 8) as usize;
        let mut object = String::from("{");
        let mut used_keys = std::collections::HashSet::new();
        let mut written = 0usize;
        while written < count {
            let key_length = (rng.next() % 12) as usize;
            let key = random_text_literal(&mut rng, key_length);
            if !used_keys.insert(key.clone()) {
                continue;
            }
            if written > 0 {
                object.push(',');
            }
            object.push_str(&key);
            object.push(':');
            match rng.next() % 4 {
                0 => object.push_str(&random_big_integer(&mut rng)),
                1 => {
                    let length = (rng.next() % 20) as usize;
                    object.push_str(&random_text_literal(&mut rng, length));
                }
                2 => object.push_str("null"),
                _ => object.push_str("[true,false,0,-0.0,1e-05]"),
            }
            written += 1;
        }
        object.push('}');
        sources.push(object);
    }
    for _ in 0..40 {
        sources.push(random_big_integer(&mut rng));
    }
    // Random decimal literals exercise Rust's correctly-rounded decimal to
    // binary64 parser against the live Python parser.
    for _ in 0..200 {
        let integer_digits = 1 + (rng.next() % 18) as usize;
        let mut number = String::new();
        if rng.next() & 1 == 0 {
            number.push('-');
        }
        number.push(char::from(b'1' + (rng.next() % 9) as u8));
        for _ in 1..integer_digits {
            number.push(char::from(b'0' + (rng.next() % 10) as u8));
        }
        let fraction_digits = (rng.next() % 20) as usize;
        if fraction_digits > 0 {
            number.push('.');
            for _ in 0..fraction_digits {
                number.push(char::from(b'0' + (rng.next() % 10) as u8));
            }
        } else if rng.next() & 1 == 0 {
            number.push_str(".0");
        }
        let exponent = (rng.next() % 501) as i64 - 250;
        number.push('e');
        if exponent >= 0 {
            number.push('+');
        }
        number.push_str(&exponent.to_string());
        sources.push(number);
    }
    sources
}

#[test]
fn structured_values_match_live_python() {
    let sources = structured_sources();
    let request: Vec<Value> = sources.iter().map(|text| json!({ "text": text })).collect();
    let input = serde_json::to_vec(&request).expect("serialize oracle request");
    let output = run_oracle("cases", &input);
    let results: Value = serde_json::from_slice(&output).expect("oracle JSON");
    let results = results.as_array().expect("oracle result array");
    assert_eq!(results.len(), sources.len());

    for (source, result) in sources.iter().zip(results) {
        let ok = result.get("ok").and_then(Value::as_bool).expect("ok");
        assert!(ok, "Python rejected source {source:?}: {result:?}");

        let expected = result
            .get("output")
            .and_then(Value::as_str)
            .expect("output");
        let parsed = parse_strict(source.as_bytes(), &Limits::default())
            .unwrap_or_else(|error| panic!("Rust rejected {source:?}: {error}"));
        let actual = canonical_dumps(&parsed, &Limits::default())
            .unwrap_or_else(|error| panic!("Rust failed to dump {source:?}: {error}"));
        assert_eq!(actual, expected, "source {source:?}");

        // Parsing the canonical ASCII form must be stable.
        let reparsed = parse_strict(actual.as_bytes(), &Limits::default()).expect("reparse");
        assert_eq!(
            canonical_dumps(&reparsed, &Limits::default()).unwrap(),
            actual,
            "unstable reparse for {source:?}"
        );
    }
}

#[test]
fn oracle_script_exists_and_is_test_only() {
    let script = oracle_script();
    let metadata = fs::metadata(&script).expect("oracle script");
    assert!(metadata.is_file());
}
