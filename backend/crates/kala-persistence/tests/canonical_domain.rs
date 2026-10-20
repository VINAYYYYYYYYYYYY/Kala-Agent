//! The larger `CanonicalJsonObject` domain, exercised beyond every tool bound.
//!
//! `kala_agent.contracts.models.CanonicalJsonObject` does not use the bounded
//! `tools.protocol` limits: strings over 32,768, keys over 256, arrays over
//! 2,048, depth over 48 and payloads over 1 MiB are all accepted. Every case
//! here is compared against the live Python contract oracle.

mod support;

use kala_json::Limits;
use kala_persistence::{
    Database, EventJournal, MAX_PERSISTENCE_JSON_DEPTH, PersistenceError, canonicalize_json_object,
    parse_json_object, persistence_limits,
};
use support::run_oracle;

fn depth_case(depth: usize) -> String {
    format!(r#"{{"x":{}0{}}}"#, "[".repeat(depth), "]".repeat(depth))
}

fn cases() -> Vec<String> {
    let mut cases: Vec<String> = Vec::new();
    // Every case below exceeds at least one `tools.protocol` bound.
    cases.push(format!(r#"{{"s":"{}"}}"#, "a".repeat(33_000)));
    cases.push(format!(r#"{{"{}":1}}"#, "k".repeat(300)));
    cases.push(format!(
        r#"{{"a":[{}]}}"#,
        std::iter::repeat_n("1", 2_100)
            .collect::<Vec<_>>()
            .join(",")
    ));
    cases.push(depth_case(49));
    cases.push(depth_case(64));
    cases.push(depth_case(200));
    cases.push(depth_case(500));
    cases.push(depth_case(900));
    cases.push(format!(r#"{{"s":"{}"}}"#, "a".repeat(1_100_000)));
    cases.push(format!(
        "{{{}}}",
        (0..2_100)
            .map(|index| format!(r#""k{index}":{index}"#))
            .collect::<Vec<_>>()
            .join(",")
    ));
    // Unicode fidelity, including a lone surrogate escape.
    cases.push(r#"{"u":"\u03ba \ud83d\ude00 \ud800"}"#.to_owned());
    cases.push(r#"{"mixed":{"b":2,"a":[1.0,-0.0,1e-05,1180591620717411303424]}}"#.to_owned());
    // CPython's default integer string-conversion limit boundary.
    cases.push(format!(r#"{{"big":{}}}"#, "9".repeat(4_300)));
    // Duplicate and non-finite values are refused by both sides.
    cases.push(r#"{"a":1,"a":2}"#.to_owned());
    cases.push(r#"{"a":NaN}"#.to_owned());
    cases.push(r#"{"a":Infinity}"#.to_owned());
    // The top level must be an object in both implementations.
    cases.push("[]".to_owned());
    cases.push(r#""text""#.to_owned());
    cases.push("1".to_owned());
    // The 4,301-digit integer literal is refused by CPython's limit.
    cases.push(format!(r#"{{"big":{}}}"#, "9".repeat(4_301)));
    cases
}

#[test]
fn persistence_limits_are_not_tool_limits() {
    let limits = persistence_limits();
    assert!(limits.max_json_bytes > Limits::DEFAULT_MAX_JSON_BYTES);
    assert!(limits.max_string_length > Limits::DEFAULT_MAX_STRING_LENGTH);
    assert!(limits.max_collection_items > Limits::DEFAULT_MAX_COLLECTION_ITEMS);
    assert!(limits.max_key_length > Limits::DEFAULT_MAX_KEY_LENGTH);
    assert!(limits.max_json_depth > Limits::DEFAULT_MAX_JSON_DEPTH);
    assert_eq!(
        limits.max_integer_digits,
        Limits::DEFAULT_MAX_INTEGER_DIGITS
    );
    assert_eq!(limits.max_json_depth, MAX_PERSISTENCE_JSON_DEPTH);
    assert_eq!(limits.max_json_depth, usize::MAX);
}

#[test]
fn accepted_and_rejected_objects_match_live_python_contract() {
    let sources = cases();
    let request = serde_json::to_vec(&sources).expect("serialize oracle request");
    let results = run_oracle(&["canonical"], &request);
    let results = results.as_array().expect("oracle result array");
    assert_eq!(results.len(), sources.len());

    let mut accepted_beyond_tool_bounds = 0usize;
    for (source, result) in sources.iter().zip(results) {
        let python_ok = result["ok"].as_bool().expect("ok flag");
        let rust = canonicalize_json_object(source);
        if python_ok {
            let expected = result["output"].as_str().expect("oracle output");
            let actual = rust.unwrap_or_else(|error| {
                panic!("Rust rejected a Python-accepted object: {error:?}")
            });
            assert_eq!(actual, expected, "canonical bytes differ");
            let reparsed = canonicalize_json_object(&actual).expect("reparse canonical");
            assert_eq!(reparsed, actual, "canonical output is not stable");
            let value = parse_json_object(&actual).expect("parse object");
            assert!(matches!(value, kala_persistence::CanonicalValue::Object(_)));
            accepted_beyond_tool_bounds += 1;
        } else {
            assert!(
                rust.is_err(),
                "Rust accepted a Python-rejected object: {source:.80}"
            );
        }
    }
    assert!(
        accepted_beyond_tool_bounds >= 13,
        "expected the large-domain cases to be accepted, got {accepted_beyond_tool_bounds}"
    );
}

#[test]
fn deep_documents_are_accepted_without_recursion() {
    // The former native ceiling was 1,024 levels. Persistence follows the
    // unbounded Python contract now; run on an ordinary 2 MiB thread to prove
    // the parser, validator and encoder do not rely on a large thread stack.
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            for depth in [1_500, 5_000] {
                let source = depth_case(depth);
                let canonical = canonicalize_json_object(&source).expect("deep object is accepted");
                assert_eq!(canonical.len(), source.len());
                let reparse = canonicalize_json_object(&canonical).expect("reparse deep object");
                assert_eq!(reparse, canonical);
            }
        })
        .expect("spawn 2 MiB thread")
        .join()
        .expect("deep canonicalization must not overflow");
}

#[test]
fn malformed_inputs_are_refused() {
    assert!(matches!(
        canonicalize_json_object("[]"),
        Err(PersistenceError::JsonObjectRequired)
    ));
    assert!(canonicalize_json_object("not json").is_err());
    assert!(canonicalize_json_object("").is_err());
    assert!(canonicalize_json_object(r#"{"a":1} trailing"#).is_err());
    assert!(kala_persistence::canonicalize_json_object_bytes(b"{\"a\":\xff}").is_err());
}

#[test]
fn journal_round_trips_a_payload_beyond_every_tool_bound() {
    let database = Database::in_memory().expect("open");
    let journal = EventJournal::new(&database).expect("journal");
    let source = format!(r#"{{"s":"{}","a":[{}]}}"#, "a".repeat(1_100_000), {
        std::iter::repeat_n("1", 2_100)
            .collect::<Vec<_>>()
            .join(",")
    });
    let canonical = canonicalize_json_object(&source).expect("canonicalize large payload");
    assert!(canonical.len() > 1_048_576);
    let event = journal
        .append("domain.large", "design-large", &source)
        .expect("append");
    assert_eq!(event.payload_json, canonical);
    assert_eq!(event.digest, kala_persistence::sha256_text(&canonical));
    let read = journal
        .get(event.sequence)
        .expect("get")
        .expect("event exists");
    assert_eq!(read, event);
    assert_eq!(
        journal.verify_chain().expect("verify").head_event_digest,
        Some(event.event_digest)
    );
}
