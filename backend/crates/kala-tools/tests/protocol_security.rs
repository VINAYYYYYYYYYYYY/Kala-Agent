//! Security tests for the tool protocol layer.
//!
//! These tests are the Rust port of `tests/security/test_schema_redos.py`.
//! A schema pattern is hostile input: a tool author registers it and every
//! tool argument is matched against it, so a pattern that can be split in
//! more than one way, or that uses a construct outside the admitted subset,
//! must be refused before it is ever compiled or run.
//!
//! The refusal must also be cheap: refusing a hostile pattern is bounded work,
//! so a registry that is fed hostile schemas cannot be used to burn CPU.

use std::time::Instant;

use kala_json::{CanonicalObject, CanonicalValue, JsonText};
use kala_tools::protocol::{
    MAX_PATTERN_INPUT_LENGTH, MAX_PATTERN_LENGTH, MAX_PATTERN_REPEAT, check_object_schema,
    validate_against_schema,
};

const ATTACK_INPUT: &str = "aaaaaaaaaaaaaaaaaaaab";

/// The repetitions that can be split in more than one way.
const AMBIGUOUS_REPETITION: [&str; 22] = [
    r"(a|aa)+$",
    r"(a|a)+$",
    r"(a|ab)*$",
    r"(|a)*",
    r"^(?:foo|bar)+$",
    r"(?:a|ab){1,64}",
    r"(a*)(a*)$",
    r"a*a*a*a*a*b",
    r"a*a*a*a*a*a*b$",
    r"[a-z]*[a-z]*[a-z]*q",
    r"\w+\s?\w+",
    r"^\w+@\w+\.\w+$",
    r"[-+]?[0-9]{1,3}",
    r"(a+)+$",
    r"(a*)+$",
    r"(?:a{1,64})*",
    r"^(a|a?)+$",
    r"(x+x+)+y",
    r"(.+)*z",
    r"(\w+\s?)*$",
    r"^(\s+)+x",
    r"(a+)*(b+)*",
];

/// The constructs outside the admitted subset.
const UNSUPPORTED_CONSTRUCTS: [&str; 18] = [
    r"(a)\1",
    r"(?P<n>a)(?P=n)",
    r"(?=x)y",
    r"(?<=x)y",
    r"(?!x)y",
    r"(?<!x)y",
    r"(a)?(?(1)b|c)",
    r"(?i)meter",
    r"(?im-sx:meter)",
    r"(?>a+)b",
    r"a*+",
    r"a++",
    r".*",
    r".+",
    r".{2,}",
    r"^.*$",
    r"(?#comment)",
    r"(?P<>a)",
];

/// The repetitions beyond the backtracking budget.
const OVER_BUDGET_REPETITION: [&str; 7] = [
    r"a{0,16}b{0,16}c{0,16}d{0,16}",
    r"a{0,32}b{0,32}c{0,32}d{0,32}",
    r"(?:ab){0,64}(?:cd){0,64}",
    r"a{0,2048}",
    r"a{0,1001}",
    r"a{100000}",
    "a{0,}a{0,}a{0,}a{0,}a{0,}a{0,}a{0,}a{0,}",
];

/// The admitted subset, with a sample that must match.
const ADMITTED_SUBSET: [(&str, &str); 14] = [
    (r"^bar-[0-9]{2}$", "bar-42"),
    (r"^[A-Za-z0-9._-]{1,64}$", "report.v1_final"),
    (r"^/[\w./-]{0,200}$", "/a/b-c.json"),
    (r"^\d{4}-\d{2}-\d{2}$", "2026-05-06"),
    (r"^(?:abc)+$", "abcabcabc"),
    (r"^(ab)+$", "abab"),
    (r"^(?:ab){2,3}$", "ababab"),
    (r"^(a+)$", "aaa"),
    (r"^[a-z]+$", "abc"),
    (r"^a?b$", "b"),
    (r"^[0-9]{1,3}$", "102"),
    (r"^\(\)$", "()"),
    (r"a|bc", "bc"),
    (r"^[^,]{0,10}$", "no commas"),
];

fn string_schema(pattern: &str, maximum: i64) -> CanonicalValue {
    let mut entries = CanonicalObject::new();
    entries.insert(JsonText::new("type"), CanonicalValue::text("string"));
    entries.insert(
        JsonText::new("maxLength"),
        CanonicalValue::Integer(maximum.into()),
    );
    entries.insert(JsonText::new("pattern"), CanonicalValue::text(pattern));
    CanonicalValue::Object(entries)
}

fn attack_input(limit: usize) -> String {
    ATTACK_INPUT.repeat(limit / ATTACK_INPUT.len() + 1)[..limit].to_owned()
}

#[test]
fn repetition_that_can_be_split_two_ways_is_refused() {
    let value = CanonicalValue::text(attack_input(MAX_PATTERN_INPUT_LENGTH));
    for pattern in AMBIGUOUS_REPETITION {
        let schema = string_schema(pattern, MAX_PATTERN_INPUT_LENGTH as i64);
        assert!(
            validate_against_schema(&value, &schema, "$", 0).is_err(),
            "pattern {pattern:?} was accepted"
        );
    }
}

#[test]
fn constructs_outside_the_admitted_subset_are_refused() {
    let value = CanonicalValue::text(attack_input(MAX_PATTERN_INPUT_LENGTH));
    for pattern in UNSUPPORTED_CONSTRUCTS {
        let schema = string_schema(pattern, MAX_PATTERN_INPUT_LENGTH as i64);
        assert!(
            validate_against_schema(&value, &schema, "$", 0).is_err(),
            "pattern {pattern:?} was accepted"
        );
    }
}

#[test]
fn repetition_beyond_the_backtracking_budget_is_refused() {
    let value = CanonicalValue::text(attack_input(MAX_PATTERN_INPUT_LENGTH));
    for pattern in OVER_BUDGET_REPETITION {
        let schema = string_schema(pattern, MAX_PATTERN_INPUT_LENGTH as i64);
        assert!(
            validate_against_schema(&value, &schema, "$", 0).is_err(),
            "pattern {pattern:?} was accepted"
        );
    }
}

#[test]
fn registration_refuses_a_schema_whose_pattern_is_ambiguous() {
    for pattern in [r"^(a|aa)+$", "a*a*a*a*a*b", r"^(\w+\s?)*$", "^(a+)+$"] {
        let mut property = CanonicalObject::new();
        property.insert(
            JsonText::new("maxLength"),
            CanonicalValue::Integer(32.into()),
        );
        property.insert(JsonText::new("pattern"), CanonicalValue::text(pattern));
        property.insert(JsonText::new("type"), CanonicalValue::text("string"));
        let mut properties = CanonicalObject::new();
        properties.insert(JsonText::new("value"), CanonicalValue::Object(property));
        let mut schema = CanonicalObject::new();
        schema.insert(
            JsonText::new("additionalProperties"),
            CanonicalValue::Bool(false),
        );
        schema.insert(
            JsonText::new("properties"),
            CanonicalValue::Object(properties),
        );
        schema.insert(
            JsonText::new("required"),
            CanonicalValue::Array(vec![CanonicalValue::text("value")]),
        );
        schema.insert(JsonText::new("type"), CanonicalValue::text("object"));
        let schema = CanonicalValue::Object(schema);
        assert!(
            check_object_schema(&schema, "schema").is_err(),
            "pattern {pattern:?} was accepted"
        );
    }
}

#[test]
fn the_admitted_subset_still_matches_and_still_binds() {
    let unmatched = "!".repeat(MAX_PATTERN_LENGTH);
    for (pattern, sample) in ADMITTED_SUBSET {
        let schema = string_schema(pattern, MAX_PATTERN_INPUT_LENGTH as i64);
        assert!(
            validate_against_schema(&CanonicalValue::text(sample), &schema, "$", 0).is_ok(),
            "pattern {pattern:?} refused its own sample"
        );
        assert!(
            validate_against_schema(&CanonicalValue::text(&unmatched), &schema, "$", 0).is_err(),
            "pattern {pattern:?} accepted an unmatched value"
        );
    }
}

#[test]
fn refusing_a_hostile_pattern_is_bounded_work() {
    let started = Instant::now();
    let value = CanonicalValue::text(attack_input(MAX_PATTERN_INPUT_LENGTH));
    for pattern in AMBIGUOUS_REPETITION {
        let schema = string_schema(pattern, MAX_PATTERN_INPUT_LENGTH as i64);
        assert!(validate_against_schema(&value, &schema, "$", 0).is_err());
    }
    for (pattern, sample) in ADMITTED_SUBSET {
        let schema = string_schema(pattern, MAX_PATTERN_INPUT_LENGTH as i64);
        assert!(validate_against_schema(&CanonicalValue::text(sample), &schema, "$", 0).is_ok());
    }
    assert!(
        started.elapsed().as_secs_f64() < 5.0,
        "refusing hostile patterns took too long"
    );
}

#[test]
fn a_pattern_needs_a_bounded_max_length() {
    for maximum in [
        None,
        Some(0),
        Some(MAX_PATTERN_INPUT_LENGTH as i64 + 1),
        Some(4_096),
    ] {
        let mut entries = CanonicalObject::new();
        entries.insert(JsonText::new("type"), CanonicalValue::text("string"));
        if let Some(maximum) = maximum {
            entries.insert(
                JsonText::new("maxLength"),
                CanonicalValue::Integer(maximum.into()),
            );
        }
        entries.insert(JsonText::new("pattern"), CanonicalValue::text("^a+$"));
        let schema = CanonicalValue::Object(entries);
        assert!(
            validate_against_schema(&CanonicalValue::text("aaa"), &schema, "$", 0).is_err(),
            "a pattern without a bounded maxLength was accepted"
        );
    }
}

#[test]
fn a_pattern_longer_than_the_bound_is_refused() {
    let pattern = "a".repeat(MAX_PATTERN_LENGTH + 1);
    let schema = string_schema(&pattern, 32);
    assert!(validate_against_schema(&CanonicalValue::text("aaa"), &schema, "$", 0).is_err());
}

#[test]
fn a_repetition_beyond_the_bound_is_refused() {
    let pattern = format!("a{{{}}}", MAX_PATTERN_REPEAT + 1);
    let schema = string_schema(&pattern, 32);
    assert!(validate_against_schema(&CanonicalValue::text("aaa"), &schema, "$", 0).is_err());
}

#[test]
fn a_refused_pattern_is_never_run() {
    // The refusal happens at schema admission, so a value that would make a
    // hostile pattern work is never matched.
    let schema = string_schema(r"(a+)+$", 64);
    let value = CanonicalValue::text("a".repeat(64));
    assert!(validate_against_schema(&value, &schema, "$", 0).is_err());
}
