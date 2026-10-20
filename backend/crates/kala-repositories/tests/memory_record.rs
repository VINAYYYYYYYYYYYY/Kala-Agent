//! `MemoryRecord` validation and transport parity.

use kala_repositories::{CanonicalJsonObject, ContractModel, MemoryRecord, Text, Timestamp};

fn record_json(content: &str, extra: &str) -> String {
    format!(
        r#"{{"memory_id":"memory-1","content":"{content}","source":"source-1","created_at":"2026-01-02T03:04:05Z"{extra}}}"#
    )
}

#[test]
fn defaults_normalization_and_wire_bytes_match_pydantic() {
    let record =
        MemoryRecord::from_json_str(&record_json("  hello  ", "")).expect("valid memory record");
    assert_eq!(record.memory_id(), "memory-1");
    assert_eq!(record.content().to_utf8().expect("utf8"), "hello");
    assert_eq!(record.source(), "source-1");
    assert_eq!(record.provenance_id(), None);
    assert_eq!(record.metadata_json().as_str(), "{}");
    assert_eq!(record.expires_at(), None);
    assert_eq!(
        record.to_wire_json().expect("wire"),
        r#"{"memory_id":"memory-1","content":"hello","source":"source-1","provenance_id":null,"metadata_json":"{}","created_at":"2026-01-02T03:04:05Z","expires_at":null}"#
    );

    let normalized = MemoryRecord::from_json_str(
        r#"{"memory_id":" \u001cmemory-2\u3000 ","content":"x","source":"\t source-2\n","provenance_id":" provenance-2 ","created_at":"2026-01-02T08:34:05+05:30","expires_at":"2026-01-02T09:34:05+05:30"}"#,
    )
    .expect("normalized record");
    assert_eq!(normalized.memory_id(), "memory-2");
    assert_eq!(normalized.source(), "source-2");
    assert_eq!(normalized.provenance_id(), Some("provenance-2"));
    assert_eq!(
        normalized.to_wire_json().expect("wire"),
        r#"{"memory_id":"memory-2","content":"x","source":"source-2","provenance_id":"provenance-2","metadata_json":"{}","created_at":"2026-01-02T08:34:05+05:30","expires_at":"2026-01-02T09:34:05+05:30"}"#
    );
    // Timestamp equality is absolute even though the wire keeps the offset.
    assert!(
        normalized.created_at() == &Timestamp::parse("2026-01-02T03:04:05Z").expect("timestamp")
    );
}

#[test]
fn extra_fields_are_forbidden_and_required_fields_are_enforced() {
    let extra = record_json("x", r#","unexpected":1"#);
    let error = MemoryRecord::from_json_str(&extra).expect_err("extra field");
    assert_eq!(error.model(), "MemoryRecord");
    assert!(
        error
            .issues()
            .iter()
            .any(|issue| issue.location == "unexpected"
                && issue.message == "Extra inputs are not permitted")
    );

    for source in [
        r#"{"memory_id":"../memory","content":"x","source":"source-1","created_at":"2026-01-02T03:04:05Z"}"#,
        r#"{"memory_id":"memory-1","content":"x","source":"../source","created_at":"2026-01-02T03:04:05Z"}"#,
        r#"{"memory_id":"memory-1","content":"x","source":"source-1","provenance_id":"../provenance","created_at":"2026-01-02T03:04:05Z"}"#,
    ] {
        let error = MemoryRecord::from_json_str(source).expect_err("unsafe identifier");
        assert!(error.message().contains("safe characters"), "{error}");
    }

    let missing = MemoryRecord::from_json_str(r#"{"memory_id":"memory-1","source":"source-1"}"#)
        .expect_err("missing fields");
    assert!(
        missing
            .issues()
            .iter()
            .any(|issue| issue.location == "content" && issue.message == "Field required")
    );
}

#[test]
fn content_matches_non_empty_text_constraints() {
    let created = Timestamp::parse("2026-01-02T03:04:05Z").expect("timestamp");
    let metadata = CanonicalJsonObject::empty();

    let max = "x".repeat(16_384);
    let record = MemoryRecord::new(
        "memory-1",
        max.as_str(),
        "source-1",
        None,
        metadata.clone(),
        created.clone(),
        None,
    )
    .expect("16384 characters");
    assert_eq!(record.content().len(), 16_384);

    for content in ["   ".to_owned(), "\u{0085}\u{00a0}".to_owned()] {
        let error = MemoryRecord::new(
            "memory-1",
            content.as_str(),
            "source-1",
            None,
            metadata.clone(),
            created.clone(),
            None,
        )
        .expect_err("empty after Pydantic trim");
        assert_eq!(error.message(), "String should have at least 1 character");
    }

    // Pydantic's strip_whitespace uses Unicode White_Space, so the C0
    // separators Python's str.strip() also handles are preserved.
    let separator = MemoryRecord::new(
        "memory-1",
        "\u{1c}",
        "source-1",
        None,
        metadata.clone(),
        created.clone(),
        None,
    )
    .expect("C0 separator is content");
    assert_eq!(separator.content().to_utf8().expect("utf8"), "\u{1c}");

    let error = MemoryRecord::new(
        "memory-1",
        "x".repeat(16_385),
        "source-1",
        None,
        metadata.clone(),
        created.clone(),
        None,
    )
    .expect_err("too long");
    assert_eq!(
        error.message(),
        "String should have at most 16384 characters"
    );

    let surrogate = Text::from_code_points(vec![0x61, 0xD800]).expect("code points");
    let error = MemoryRecord::new(
        "memory-1", surrogate, "source-1", None, metadata, created, None,
    )
    .expect_err("surrogate");
    assert!(error.message().contains("unicode string"));
}

#[test]
fn expiry_must_be_strictly_later_as_an_absolute_instant() {
    let created = Timestamp::parse("2026-01-02T03:04:05Z").expect("timestamp");
    let metadata = CanonicalJsonObject::empty();
    let equal_utc = Timestamp::parse("2026-01-02T08:34:05+05:30").expect("timestamp");
    assert_eq!(created, equal_utc);

    let error = MemoryRecord::new(
        "memory-1",
        "content",
        "source-1",
        None,
        metadata.clone(),
        created.clone(),
        Some(equal_utc.clone()),
    )
    .expect_err("equal expiry");
    assert_eq!(error.message(), "expires_at must be later than created_at");

    let earlier = Timestamp::parse("2026-01-02T03:04:04.999999Z").expect("timestamp");
    assert!(
        MemoryRecord::new(
            "memory-1",
            "content",
            "source-1",
            None,
            metadata.clone(),
            created.clone(),
            Some(earlier),
        )
        .is_err()
    );

    let later = Timestamp::parse("2026-01-02T03:04:05.000001Z").expect("timestamp");
    MemoryRecord::new(
        "memory-1",
        "content",
        "source-1",
        None,
        metadata.clone(),
        created.clone(),
        Some(later),
    )
    .expect("one microsecond later");

    let json_error = MemoryRecord::from_json_str(
        r#"{"memory_id":"memory-1","content":"x","source":"source-1","created_at":"2026-01-02T08:34:05+05:30","expires_at":"2026-01-02T03:04:05Z"}"#,
    )
    .expect_err("cross-offset equal expiry");
    assert_eq!(
        json_error.message(),
        "expires_at must be later than created_at"
    );
}

#[test]
fn metadata_is_canonical_and_duplicate_rules_stay_distinct() {
    let record = MemoryRecord::from_json_str(
        r#"{"memory_id":"memory-1","content":"x","source":"source-1","metadata_json":"{\"z\":1,\"a\":{\"b\":2,\"a\":1}}","created_at":"2026-01-02T03:04:05Z"}"#,
    )
    .expect("canonical metadata");
    assert_eq!(
        record.metadata_json().as_str(),
        r#"{"a":{"a":1,"b":2},"z":1}"#
    );

    let object_metadata = MemoryRecord::from_json_str(
        r#"{"memory_id":"memory-1","content":"x","source":"source-1","metadata_json":{"z":2,"a":1},"created_at":"2026-01-02T03:04:05Z"}"#,
    )
    .expect("object metadata input");
    assert_eq!(object_metadata.metadata_json().as_str(), r#"{"a":1,"z":2}"#);

    let null_metadata = MemoryRecord::from_json_str(
        r#"{"memory_id":"memory-1","content":"x","source":"source-1","metadata_json":null,"created_at":"2026-01-02T03:04:05Z"}"#,
    )
    .expect_err("null metadata is not an object");
    assert!(null_metadata.message().contains("JSON object"));

    let duplicate_metadata = MemoryRecord::from_json_str(
        r#"{"memory_id":"memory-1","content":"x","source":"source-1","metadata_json":"{\"a\":1,\"a\":2}","created_at":"2026-01-02T03:04:05Z"}"#,
    )
    .expect_err("duplicate metadata keys are strict");
    assert!(duplicate_metadata.message().contains("JSON object"));

    // Pydantic model JSON itself is last-wins, while canonical metadata text is
    // strict. The distinction is preserved by using the accepted model parser.
    let last_wins = MemoryRecord::from_json_str(
        r#"{"memory_id":"memory-1","content":"first","content":"second","source":"source-1","created_at":"2026-01-02T03:04:05Z"}"#,
    )
    .expect("outer duplicate last wins");
    assert_eq!(last_wins.content().to_utf8().expect("utf8"), "second");

    let last_wins_metadata = MemoryRecord::from_json_str(
        r#"{"memory_id":"memory-1","content":"x","source":"source-1","metadata_json":"{}","metadata_json":"{\"a\":1}","created_at":"2026-01-02T03:04:05Z"}"#,
    )
    .expect("duplicate field last wins");
    assert_eq!(last_wins_metadata.metadata_json().as_str(), r#"{"a":1}"#);
}

#[test]
fn metadata_keeps_arbitrary_integers_and_surrogate_escapes() {
    let big = "7".repeat(4_000);
    let source = format!(
        r#"{{"memory_id":"memory-1","content":"x","source":"source-1","metadata_json":"{{\"big\":{big},\"text\":\"\\ud800\"}}","created_at":"2026-01-02T03:04:05Z"}}"#
    );
    let record = MemoryRecord::from_json_str(&source).expect("large integer metadata");
    let metadata = record.metadata_json().to_value().expect("metadata value");
    let object = match &metadata {
        kala_json::CanonicalValue::Object(object) => object,
        _ => panic!("metadata object"),
    };
    let big_value = object
        .get(&kala_json::JsonText::new("big"))
        .expect("big field")
        .as_integer()
        .expect("integer");
    assert_eq!(big_value.to_string(), big);
    assert_eq!(record.metadata_json().as_str().matches(&big).count(), 1);

    // The surrogate lives inside canonical JSON escapes, which is representable
    // text, not a raw lone surrogate in a contract string field.
    assert!(
        record
            .metadata_json()
            .as_str()
            .contains(r#""text":"\ud800""#)
    );
    assert_eq!(
        MemoryRecord::from_json_str(&record.to_wire_json().expect("wire")).expect("round trip"),
        record
    );
}

#[test]
fn deep_and_large_metadata_use_the_unbounded_persistence_domain() {
    let mut deep = "0".to_owned();
    for _ in 0..80 {
        deep = format!("[{deep}]");
    }
    let deep_source = format!(r#"{{"deep":{deep}}}"#);
    let deep_metadata = CanonicalJsonObject::parse(&deep_source).expect("deep metadata");
    assert_eq!(deep_metadata.as_str(), deep_source);

    let created = Timestamp::parse("2026-01-02T03:04:05Z").expect("timestamp");
    let deep_record = MemoryRecord::new(
        "memory-deep",
        "deep",
        "source-1",
        None,
        deep_metadata,
        created.clone(),
        None,
    )
    .expect("deep record");
    let reparsed =
        MemoryRecord::from_json_str(&deep_record.to_wire_json().expect("wire")).expect("reparse");
    assert_eq!(reparsed, deep_record);

    // The Python contract's CanonicalJsonObject has no 1 MiB cap. This string
    // is larger than the bounded tools.protocol payload limit on purpose.
    let large_source = format!(r#"{{"large":"{}"}}"#, "a".repeat(1_100_000));
    let large_metadata = CanonicalJsonObject::parse(&large_source).expect("large metadata");
    assert!(large_metadata.as_str().len() > 1_048_576);
    let large_record = MemoryRecord::new(
        "memory-large",
        "large",
        "source-1",
        None,
        large_metadata,
        created,
        None,
    )
    .expect("large record");
    let wire = large_record.to_wire_json().expect("wire");
    assert!(wire.len() > 1_048_576);
    let reparsed = MemoryRecord::from_json_str(&wire).expect("reparse large");
    assert_eq!(reparsed, large_record);
}
