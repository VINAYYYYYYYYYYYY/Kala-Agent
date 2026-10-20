//! Consumes the real Python-generated compatibility corpus.
//!
//! These tests never contain hand-written expected digest bytes: every success
//! output, digest and error message is read from
//! `backend/fixtures/corpus/v1`. The corpus is owned by the Python corpus
//! worker and is consumed read-only here.

use std::fs;
use std::path::PathBuf;

use kala_json::{
    CanonicalJsonError, CanonicalValue, EventChainFields, JsonText, Limits, Sha256Digest,
    canonical_dumps, chain_digest, parse_strict, payload_digest, verify_chain,
};
use num_bigint::BigInt;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus/v1")
}

fn load(name: &str) -> Value {
    let path = corpus_dir().join(name);
    let bytes = fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn required_str<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing string field {key}"))
}

fn optional_str(value: &Value, key: &str) -> Option<String> {
    match value.get(key) {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Null) | None => None,
        other => panic!("field {key} is not a string or null: {other:?}"),
    }
}

fn required_u64(value: &Value, key: &str) -> u64 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("missing integer field {key}"))
}

fn repeated(character: &str, count: usize) -> String {
    std::iter::repeat_n(character, count).collect()
}

/// Materialize a canonical-JSON dumps fixture source.
///
/// Three Python-only source kinds have no typed Rust equivalent. They are
/// asserted explicitly instead of being skipped: a non-string mapping key and
/// a raw `Decimal` are outside `CanonicalValue`'s type domain (honest
/// compile-time refusal), and a non-finite `float` is constructed directly and
/// must be refused by the serializer.
fn materialize_dumps_source(source: &Value) -> Result<CanonicalValue, CanonicalJsonError> {
    let kind = required_str(source, "kind");
    match kind {
        "json_text" => {
            let text = required_str(source, "text");
            parse_strict(text.as_bytes(), &Limits::default())
        }
        "utf8_text" => Ok(CanonicalValue::text(required_str(source, "text"))),
        "repeat_string" => Ok(CanonicalValue::text(repeated(
            required_str(source, "char"),
            required_u64(source, "count") as usize,
        ))),
        "nested_arrays" => {
            let levels = required_u64(source, "levels") as usize;
            let mut value = CanonicalValue::Integer(BigInt::from(0));
            for _ in 0..levels {
                value = CanonicalValue::Array(vec![value]);
            }
            Ok(value)
        }
        "array_of_strings" => {
            let item = required_str(source, "item");
            let count = required_u64(source, "count") as usize;
            Ok(CanonicalValue::Array(
                (0..count).map(|_| CanonicalValue::text(item)).collect(),
            ))
        }
        "array_of_repeated_strings" => {
            let item = repeated(
                required_str(source, "char"),
                required_u64(source, "item_count") as usize,
            );
            let count = required_u64(source, "array_count") as usize;
            Ok(CanonicalValue::Array(
                (0..count)
                    .map(|_| CanonicalValue::text(item.as_str()))
                    .collect(),
            ))
        }
        "array_of_ints" => {
            let count = required_u64(source, "count") as usize;
            Ok(CanonicalValue::Array(vec![
                CanonicalValue::Integer(
                    BigInt::from(0)
                );
                count
            ]))
        }
        "object_key_length" => {
            let key = repeated(
                required_str(source, "char"),
                required_u64(source, "count") as usize,
            );
            Ok(CanonicalValue::object([(
                key,
                CanonicalValue::Integer(BigInt::from(1)),
            )]))
        }
        "float_literal" => {
            let literal = required_str(source, "literal");
            let value = match literal {
                "nan" => f64::NAN,
                "inf" => f64::INFINITY,
                "-inf" => f64::NEG_INFINITY,
                other => panic!("unexpected float literal {other}"),
            };
            Ok(CanonicalValue::Float(value))
        }
        other => panic!("unsupported fixture source kind {other}"),
    }
}

fn check_dumps_case(case: &Value) {
    let id = required_str(case, "id");
    let source = case.get("input").expect("input");
    let expected = case.get("expected").expect("expected");
    let expected_ok = expected
        .get("ok")
        .and_then(Value::as_bool)
        .expect("expected.ok");

    if required_str(source, "kind") == "object_nonstring_key" {
        assert!(
            !expected_ok,
            "{id}: fixture unexpectedly accepts a non-string key"
        );
        assert_eq!(
            expected.get("message").and_then(Value::as_str),
            Some(CanonicalJsonError::NonStringKey.to_string().as_str()),
            "{id}: non-string-key error vocabulary drifted"
        );
        return;
    }
    if required_str(source, "kind") == "decimal_literal" {
        assert!(!expected_ok, "{id}: fixture unexpectedly accepts a Decimal");
        assert_eq!(
            expected.get("message").and_then(Value::as_str),
            Some(CanonicalJsonError::NotSerializable.to_string().as_str()),
            "{id}: Decimal error vocabulary drifted"
        );
        return;
    }

    let limits = match case.get("max_bytes").and_then(Value::as_u64) {
        Some(max_bytes) => Limits {
            max_json_bytes: max_bytes as usize,
            ..Limits::default()
        },
        None => Limits::default(),
    };
    let value = match materialize_dumps_source(source) {
        Ok(value) => value,
        Err(error) => {
            assert!(
                !expected_ok,
                "{id}: unexpected materialization error {error}"
            );
            assert_eq!(
                expected.get("message").and_then(Value::as_str),
                Some(error.to_string().as_str()),
                "{id}: error message mismatch"
            );
            return;
        }
    };
    match canonical_dumps(&value, &limits) {
        Ok(output) => {
            assert!(expected_ok, "{id}: expected failure but dumped {output:?}");
            if let Some(expected_output) = expected.get("output").and_then(Value::as_str) {
                assert_eq!(output, expected_output, "{id}: canonical output mismatch");
            }
            assert_eq!(
                output.len() as u64,
                required_u64(expected, "output_length"),
                "{id}: output length mismatch"
            );
            assert_eq!(
                sha256_hex(output.as_bytes()),
                required_str(expected, "output_sha256"),
                "{id}: output digest mismatch"
            );
            assert_eq!(
                sha256_hex(output.as_bytes()),
                required_str(expected, "digest"),
                "{id}: digest field mismatch"
            );
        }
        Err(error) => {
            assert!(!expected_ok, "{id}: expected success but failed: {error}");
            assert_eq!(
                expected.get("message").and_then(Value::as_str),
                Some(error.to_string().as_str()),
                "{id}: error message mismatch"
            );
        }
    }
}

fn parse_hex(text: &str) -> Vec<u8> {
    assert!(text.len() % 2 == 0, "odd-length hex fixture");
    text.as_bytes()
        .chunks_exact(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).expect("hex digit"))
        .collect()
}

fn check_loads_case(case: &Value) {
    let id = required_str(case, "id");
    let source = case.get("input").expect("input");
    let expected = case.get("expected").expect("expected");
    let expected_ok = expected
        .get("ok")
        .and_then(Value::as_bool)
        .expect("expected.ok");
    let payload = match required_str(source, "kind") {
        "hex" => parse_hex(required_str(source, "hex")),
        "quoted_repeat" => {
            let mut text = String::from("\"");
            text.push_str(&repeated(
                required_str(source, "char"),
                required_u64(source, "count") as usize,
            ));
            text.push('"');
            text.into_bytes()
        }
        _ => required_str(source, "text").as_bytes().to_vec(),
    };

    match parse_strict(&payload, &Limits::default()) {
        Ok(value) => {
            assert!(expected_ok, "{id}: expected failure but parsed {value:?}");
            let canonical = canonical_dumps(&value, &Limits::default()).expect("dump");
            if let Some(expected_canonical) = expected.get("canonical").and_then(Value::as_str) {
                assert_eq!(canonical, expected_canonical, "{id}: canonical mismatch");
            }
            assert_eq!(
                canonical.len() as u64,
                required_u64(expected, "canonical_length"),
                "{id}: canonical length mismatch"
            );
            assert_eq!(
                sha256_hex(canonical.as_bytes()),
                required_str(expected, "canonical_sha256"),
                "{id}: canonical digest mismatch"
            );
        }
        Err(error) => {
            assert!(!expected_ok, "{id}: expected success but failed: {error}");
            assert_eq!(
                expected.get("message").and_then(Value::as_str),
                Some(error.to_string().as_str()),
                "{id}: error message mismatch"
            );
        }
    }
}

#[test]
fn canonical_json_corpus_byte_parity() {
    let fixture = load("canonical_json.json");
    let cases = fixture
        .get("cases")
        .and_then(Value::as_array)
        .expect("cases");
    assert_eq!(cases.len(), 66, "unexpected canonical JSON case count");
    for case in cases {
        match required_str(case, "operation") {
            "dumps" => check_dumps_case(case),
            "loads" => check_loads_case(case),
            other => panic!("unknown operation {other}"),
        }
    }
}

fn fields_from_row(row: &Value) -> EventChainFields {
    EventChainFields {
        sequence: required_u64(row, "sequence"),
        event_type: required_str(row, "event_type").to_owned(),
        aggregate_id: required_str(row, "aggregate_id").to_owned(),
        workflow_id: optional_str(row, "workflow_id"),
        run_id: optional_str(row, "run_id"),
        occurred_at: required_str(row, "occurred_at").to_owned(),
        payload_digest: Sha256Digest::from_hex(required_str(row, "digest"))
            .expect("payload digest hex"),
        previous_event_digest: Sha256Digest::from_hex(required_str(row, "previous_event_digest"))
            .expect("previous digest hex"),
    }
}

#[test]
fn event_chain_corpus_and_sqlite_rows_parity() {
    let fixture = load("event_chain.json");
    let chain = fixture.get("chain").expect("chain");
    assert_eq!(required_str(chain, "domain"), kala_json::CHAIN_DOMAIN);
    assert_eq!(
        required_str(chain, "genesis_previous_digest"),
        Sha256Digest::ZERO.to_hex()
    );
    assert_eq!(required_u64(chain, "length_prefix_bytes"), 8);
    assert_eq!(required_str(chain, "length_prefix_byte_order"), "big");

    let preimage_cases = fixture
        .get("preimage_cases")
        .and_then(Value::as_array)
        .expect("preimage cases");
    assert_eq!(preimage_cases.len(), 5);
    for case in preimage_cases {
        let id = required_str(case, "id");
        let values = case.get("fields").expect("fields");
        let fields = EventChainFields {
            sequence: required_u64(values, "sequence"),
            event_type: required_str(values, "event_type").to_owned(),
            aggregate_id: required_str(values, "aggregate_id").to_owned(),
            workflow_id: optional_str(values, "workflow_id"),
            run_id: optional_str(values, "run_id"),
            occurred_at: required_str(values, "occurred_at").to_owned(),
            payload_digest: Sha256Digest::from_hex(required_str(values, "payload_digest"))
                .expect("payload digest"),
            previous_event_digest: Sha256Digest::from_hex(required_str(
                values,
                "previous_event_digest",
            ))
            .expect("previous digest"),
        };
        let expected_parts: Vec<String> = case
            .get("as_preimage_parts")
            .and_then(Value::as_array)
            .expect("preimage parts")
            .iter()
            .map(|part| part.as_str().expect("part string").to_owned())
            .collect();
        assert_eq!(
            fields.preimage_parts().to_vec(),
            expected_parts,
            "{id}: parts"
        );
        assert_eq!(
            chain_digest(&fields).to_hex(),
            required_str(case, "chain_digest"),
            "{id}: chain digest"
        );
    }

    // The two optional-id cases must collapse to the same digest.
    assert_eq!(
        required_str(&preimage_cases[0], "chain_digest"),
        required_str(&preimage_cases[1], "chain_digest")
    );

    let database = fixture.get("database").expect("database");
    let event_rows = database
        .get("events")
        .and_then(Value::as_array)
        .expect("events");
    assert_eq!(event_rows.len(), 5);
    let mut events = Vec::new();
    let mut stored_digests = Vec::new();
    for row in event_rows {
        let payload_json = required_str(row, "payload_json");
        assert_eq!(
            payload_digest(payload_json).to_hex(),
            required_str(row, "digest"),
            "payload digest mismatch at sequence {}",
            required_u64(row, "sequence")
        );

        // Stored payload bytes must already be canonical. This is also the
        // real corpus check for bigint, -0.0, exponent-threshold and surrogate
        // escaping behaviour inside a payload.
        let parsed = parse_strict(payload_json.as_bytes(), &Limits::default())
            .unwrap_or_else(|error| panic!("payload is not strict JSON: {error}"));
        let recanonicalized = canonical_dumps(&parsed, &Limits::default()).unwrap();
        assert_eq!(
            recanonicalized, payload_json,
            "payload bytes are not canonical"
        );

        let fields = fields_from_row(row);
        let stored =
            Sha256Digest::from_hex(required_str(row, "event_digest")).expect("event digest hex");
        assert_eq!(
            chain_digest(&fields).to_hex(),
            stored.to_hex(),
            "chain digest"
        );
        stored_digests.push(stored);
        events.push(fields);
    }

    let verification = verify_chain(&events, Some(&stored_digests)).expect("verify chain");
    assert_eq!(
        verification.event_count,
        required_u64(database, "verified_event_count")
    );
    assert_eq!(
        verification.head_sequence,
        Some(required_u64(database, "verified_head_sequence"))
    );
    assert_eq!(
        verification.head_event_digest.map(|digest| digest.to_hex()),
        Some(required_str(database, "verified_head_event_digest").to_owned())
    );

    let snapshot = database.get("snapshot").expect("snapshot");
    let state_json = required_str(snapshot, "state_json");
    let parsed_state = parse_strict(state_json.as_bytes(), &Limits::default()).expect("state");
    assert_eq!(
        canonical_dumps(&parsed_state, &Limits::default()).unwrap(),
        state_json
    );
    assert_eq!(
        sha256_hex(state_json.as_bytes()),
        required_str(snapshot, "digest"),
        "snapshot digest"
    );
}

#[test]
fn typed_value_rejects_python_only_domains_honestly() {
    // A non-string key and a raw Decimal cannot be represented; the fixture
    // errors are part of the checked compatibility vocabulary above. This test
    // documents that the refusal is by type, not a silent coercion.
    assert_eq!(
        CanonicalJsonError::NonStringKey.to_string(),
        "JSON object keys must be strings"
    );
    assert_eq!(
        CanonicalJsonError::NotSerializable.to_string(),
        "value is not JSON serializable"
    );
    let text = JsonText::from_code_points(vec![0xD800]).expect("lone surrogate is representable");
    assert!(text.has_surrogates());
    assert!(text.to_utf8().is_err());
}
