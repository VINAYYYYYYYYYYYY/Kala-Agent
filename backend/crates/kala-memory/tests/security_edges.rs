//! Security and edge-case tests for the native memory manager.
//!
//! The cases here are the ones an attacker can reach: identifier handling at
//! every lookup boundary, content and metadata limits, the bounded working
//! map, the bounded durable scan, expiry, and concurrent access.

use kala_contracts::{CanonicalJsonObject, Text, Timestamp};
use kala_json::CanonicalValue;
use kala_memory::{
    AddOptions, DURABLE_RECORD_SCAN_LIMIT, GetOptions, ListOptions, ListWorkingOptions,
    MemoryError, MemoryManager, MemoryManagerConfig, MemoryRecord, MemoryRepository,
    PromoteOptions, PromoteTarget, new_record,
};
use kala_persistence::Database;

const RECORDED_AT: &str = "2026-01-02T03:04:05Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse(value).expect("timestamp")
}

fn record(
    memory_id: &str,
    content: &str,
    source: &str,
    created_at: &str,
    expires_at: Option<&str>,
) -> MemoryRecord {
    new_record(
        memory_id,
        content,
        source,
        None,
        CanonicalJsonObject::empty(),
        timestamp(created_at),
        expires_at.map(timestamp),
    )
    .expect("record")
}

fn manager(max_working_records: usize) -> MemoryManager<'static> {
    let database: &'static Database = Box::leak(Box::new(Database::in_memory().expect("database")));
    MemoryManager::with_config(
        MemoryRepository::new(database),
        MemoryManagerConfig::new().with_max_working_records(max_working_records),
    )
    .expect("manager")
}

// ---------------------------------------------------------------------------
// Identifiers.
// ---------------------------------------------------------------------------

#[test]
fn identifiers_are_validated_at_every_lookup_boundary() {
    let manager = manager(16);
    let stored = record("m-1", "alpha", "source-1", RECORDED_AT, None);
    manager
        .add(
            &stored,
            AddOptions {
                durable: false,
                promote: true,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");

    for hostile in ["../memory", "m-1/../m-2", "m 1", "m-1!", "-m-1"] {
        let error = manager
            .get(
                hostile,
                GetOptions {
                    now: Some(timestamp(RECORDED_AT)),
                    include_expired: false,
                },
            )
            .expect_err("hostile identifier");
        assert_eq!(error.python_class_name(), "ValueError", "{hostile}");
        assert_eq!(
            error.message(),
            "memory_id must be a safe identifier",
            "{hostile}"
        );
    }

    let error = manager
        .list(
            ListOptions::new()
                .with_source("../escape")
                .with_now(timestamp(RECORDED_AT)),
        )
        .expect_err("hostile source");
    assert_eq!(error.python_class_name(), "ValueError");
    assert_eq!(error.message(), "source must be a safe identifier");

    let error = manager.delete("../escape").expect_err("hostile delete");
    assert_eq!(error.python_class_name(), "ValueError");
    assert_eq!(error.message(), "memory_id must be a safe identifier");
}

#[test]
fn an_empty_identifier_is_refused_before_the_repository() {
    let manager = manager(16);
    for hostile in ["", "   ", "\t\n"] {
        let error = manager
            .get(
                hostile,
                GetOptions {
                    now: Some(timestamp(RECORDED_AT)),
                    include_expired: false,
                },
            )
            .expect_err("blank identifier");
        assert_eq!(error.message(), "memory_id must be nonempty");
    }
}

#[test]
fn the_working_map_uses_the_raw_identifier_and_the_repository_normalizes_it() {
    // The Python manager looks the working map up with the raw string but
    // hands the repository a normalized one, so a padded identifier removes
    // the durable row and leaves the working entry behind.
    let manager = manager(16);
    let stored = record("m-1", "alpha", "source-1", RECORDED_AT, None);
    manager
        .add(
            &stored,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    manager.repository().put(&stored).expect("durable put");
    assert!(manager.delete(" m-1 ").expect("delete"));
    // The durable row is gone, but the working entry is still keyed by the
    // raw identifier, so the record is still readable from working memory.
    assert!(
        manager
            .get(
                "m-1",
                GetOptions {
                    now: Some(timestamp(RECORDED_AT)),
                    include_expired: false,
                },
            )
            .expect("get")
            .is_some()
    );
    assert_eq!(
        manager
            .list_working(
                ListWorkingOptions::new()
                    .with_now(timestamp(RECORDED_AT))
                    .with_limit(10),
            )
            .expect("list")
            .len(),
        1
    );
}

// ---------------------------------------------------------------------------
// Content and metadata.
// ---------------------------------------------------------------------------

#[test]
fn content_is_trimmed_and_bounded() {
    let padded = record("m-1", "  alpha  ", "source-1", RECORDED_AT, None);
    assert_eq!(padded.content().to_string(), "alpha");
    let empty = new_record(
        "m-2",
        "   ",
        "source-1",
        None,
        CanonicalJsonObject::empty(),
        timestamp(RECORDED_AT),
        None,
    )
    .expect_err("blank content");
    assert_eq!(empty.python_class_name(), "ValidationError");
    assert_eq!(empty.message(), "String should have at least 1 character");

    let long = "a".repeat(16_384);
    let at_limit = new_record(
        "m-3",
        &long,
        "source-1",
        None,
        CanonicalJsonObject::empty(),
        timestamp(RECORDED_AT),
        None,
    )
    .expect("16,384 code points are allowed");
    assert_eq!(at_limit.content().len(), 16_384);

    let too_long = new_record(
        "m-4",
        &"a".repeat(16_385),
        "source-1",
        None,
        CanonicalJsonObject::empty(),
        timestamp(RECORDED_AT),
        None,
    )
    .expect_err("16,385 code points are refused");
    assert_eq!(
        too_long.message(),
        "String should have at most 16384 characters"
    );
}

#[test]
fn content_is_trimmed_by_code_point_not_by_byte() {
    // Sixteen thousand astral code points are 64,000 UTF-8 bytes but only
    // 16,000 code points, so the content limit accepts them.
    let content: String = "\u{1F600}".repeat(16_000);
    let record = new_record(
        "m-1",
        &content,
        "source-1",
        None,
        CanonicalJsonObject::empty(),
        timestamp(RECORDED_AT),
        None,
    )
    .expect("astral content");
    assert_eq!(record.content().len(), 16_000);
}

#[test]
fn metadata_must_be_a_canonical_json_object() {
    let manager = manager(16);
    let metadata = CanonicalJsonObject::parse(r#"{"b":1,"a":2}"#).expect("canonical");
    assert_eq!(metadata.as_str(), r#"{"a":2,"b":1}"#);
    let record = new_record(
        "m-1",
        "alpha",
        "source-1",
        None,
        metadata,
        timestamp(RECORDED_AT),
        None,
    )
    .expect("record");
    manager
        .add(
            &record,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let listed = manager
        .list(
            ListOptions::new()
                .with_now(timestamp(RECORDED_AT))
                .with_limit(10),
        )
        .expect("list");
    assert_eq!(listed[0].metadata_json().as_str(), r#"{"a":2,"b":1}"#);
}

#[test]
fn deeply_nested_metadata_does_not_overflow_the_stack() {
    // The persistence domain has no depth cap, and the canonical JSON
    // parser and serializer are iterative, so an attacker-sized nesting does
    // not recurse.
    let depth = 20_000;
    let mut json = String::from("[");
    json.push_str(&"[".repeat(depth - 1));
    json.push_str(&"]".repeat(depth));
    let metadata =
        CanonicalJsonObject::parse(&format!("{{\"nested\":{json}}}")).expect("deep metadata");
    assert!(metadata.as_str().starts_with("{\"nested\":[["));
}

// ---------------------------------------------------------------------------
// Bounds.
// ---------------------------------------------------------------------------

#[test]
fn the_working_map_is_bounded() {
    let manager = manager(8);
    for index in 0..8 {
        manager
            .add(
                &record(
                    &format!("m-{index}"),
                    &format!("content-{index}"),
                    &format!("source-{index}"),
                    RECORDED_AT,
                    None,
                ),
                AddOptions {
                    durable: false,
                    promote: false,
                    now: Some(timestamp(RECORDED_AT)),
                },
            )
            .expect("add");
    }
    let error = manager
        .add(
            &record("m-8", "content-8", "source-8", RECORDED_AT, None),
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect_err("bounded");
    assert_eq!(error.python_class_name(), "MemoryPromotionError");
    assert_eq!(error.message(), "working memory limit exceeded");
}

#[test]
fn the_durable_scan_is_bounded() {
    assert_eq!(DURABLE_RECORD_SCAN_LIMIT, 1_000);
    let manager = manager(4);
    // The thousand durable records of the source share one content, so the
    // candidate matches them. The thousand-and-first durable record holds a
    // different content, but it is outside the window the manager reads, so
    // the candidate is reported as a duplicate rather than a conflict.
    for index in 0..1_000 {
        manager
            .repository()
            .put(&record(
                &format!("d-{index:04}"),
                "shared",
                "source-1",
                RECORDED_AT,
                None,
            ))
            .expect("durable put");
    }
    manager
        .repository()
        .put(&record("d-1000", "other", "source-1", RECORDED_AT, None))
        .expect("durable put");
    let result = manager
        .try_add(
            &record("probe", "shared", "source-1", RECORDED_AT, None),
            Some(timestamp(RECORDED_AT)),
        )
        .expect("try_add");
    assert_eq!(result.status(), kala_memory::MemoryStatus::Duplicate);
    assert_eq!(result.existing().expect("existing").memory_id(), "d-0000");
}

#[test]
fn a_list_limit_must_be_positive() {
    let manager = manager(4);
    for limit in [0, -1, i64::MIN] {
        let error = manager
            .list(ListOptions::new().with_limit(limit))
            .expect_err("limit");
        assert_eq!(error.python_class_name(), "ValueError");
        assert_eq!(error.message(), "limit must be positive");
    }
}

#[test]
fn the_working_list_uses_python_slice_semantics() {
    let manager = manager(8);
    for index in 0..3 {
        manager
            .add(
                &record(
                    &format!("m-{index}"),
                    &format!("content-{index}"),
                    &format!("source-{index}"),
                    RECORDED_AT,
                    None,
                ),
                AddOptions {
                    durable: false,
                    promote: false,
                    now: Some(timestamp(RECORDED_AT)),
                },
            )
            .expect("add");
    }
    let listed = |limit: i64| -> Vec<String> {
        manager
            .list_working(
                ListWorkingOptions::new()
                    .with_now(timestamp(RECORDED_AT))
                    .with_limit(limit),
            )
            .expect("list")
            .iter()
            .map(|item| item.memory_id().to_owned())
            .collect()
    };
    assert_eq!(listed(0), Vec::<String>::new());
    assert_eq!(listed(1), ["m-0"]);
    assert_eq!(listed(-1), ["m-0", "m-1"]);
    assert_eq!(listed(-5), Vec::<String>::new());
    assert_eq!(listed(1_000), ["m-0", "m-1", "m-2"]);
}

// ---------------------------------------------------------------------------
// Expiry.
// ---------------------------------------------------------------------------

#[test]
fn an_expired_durable_record_is_hidden_but_can_be_promoted_again() {
    let manager = manager(8);
    let expiring = record(
        "m-1",
        "alpha",
        "source-1",
        RECORDED_AT,
        Some("2026-01-02T03:05:05Z"),
    );
    manager
        .promote(
            PromoteTarget::Record(&expiring),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    let later = "2026-01-02T03:06:05Z";
    assert!(
        manager
            .get(
                "m-1",
                GetOptions {
                    now: Some(timestamp(later)),
                    include_expired: false,
                },
            )
            .expect("get")
            .is_none()
    );
    assert!(
        manager
            .get(
                "m-1",
                GetOptions {
                    now: Some(timestamp(later)),
                    include_expired: true,
                },
            )
            .expect("get")
            .is_some()
    );
    // A promotion reads the durable record even when it has expired.
    let promoted = manager
        .promote(
            PromoteTarget::Id("m-1"),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(later)),
            },
        )
        .expect("promote");
    assert_eq!(promoted.memory_id(), "m-1");
}

#[test]
fn an_expired_record_does_not_block_a_new_content_for_the_same_source() {
    let manager = manager(8);
    let expiring = record(
        "m-1",
        "alpha",
        "source-1",
        RECORDED_AT,
        Some("2026-01-02T03:05:05Z"),
    );
    manager
        .promote(
            PromoteTarget::Record(&expiring),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    let later = "2026-01-02T03:06:05Z";
    let result = manager
        .try_add(
            &record("m-2", "beta", "source-1", RECORDED_AT, None),
            Some(timestamp(later)),
        )
        .expect("try_add");
    assert_eq!(result.status(), kala_memory::MemoryStatus::Added);
}

// ---------------------------------------------------------------------------
// Promotion guards.
// ---------------------------------------------------------------------------

#[test]
fn an_invalid_provenance_override_is_refused_before_the_store_is_touched() {
    let manager = manager(8);
    let stored = record("m-1", "alpha", "source-1", RECORDED_AT, None);
    manager
        .add(
            &stored,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let error = manager
        .promote(
            PromoteTarget::Id("m-1"),
            PromoteOptions {
                provenance_id: Some("../escape".to_owned()),
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect_err("provenance");
    assert_eq!(error.python_class_name(), "ValidationError");
    assert_eq!(
        error.message(),
        "identifier must be nonempty and contain only safe characters"
    );
    assert!(
        manager
            .list_durable(
                ListOptions::new()
                    .with_now(timestamp(RECORDED_AT))
                    .with_limit(10),
            )
            .expect("list")
            .is_empty()
    );
}

#[test]
fn a_promotion_can_require_provenance() {
    let database: &'static Database = Box::leak(Box::new(Database::in_memory().expect("database")));
    let manager = MemoryManager::with_config(
        MemoryRepository::new(database),
        MemoryManagerConfig::new().with_require_provenance_on_promotion(true),
    )
    .expect("manager");
    let error = manager
        .promote(
            PromoteTarget::Record(&record("m-1", "alpha", "source-1", RECORDED_AT, None)),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect_err("provenance");
    assert_eq!(error.python_class_name(), "MemoryPromotionError");
    assert_eq!(error.message(), "durable memory requires provenance");
}

// ---------------------------------------------------------------------------
// Projections.
// ---------------------------------------------------------------------------

#[test]
fn the_add_result_projection_uses_the_model_field_order() {
    let manager = manager(8);
    let stored = record("m-1", "alpha", "source-1", RECORDED_AT, None);
    let result = manager
        .try_add(&stored, Some(timestamp(RECORDED_AT)))
        .expect("try_add");
    let payload = result.as_dict();
    let record_value = match &payload {
        CanonicalValue::Object(object) => object
            .get(&kala_json::JsonText::new("record"))
            .cloned()
            .expect("record"),
        _ => panic!("object"),
    };
    let rendered = kala_json::canonical_dumps(&record_value, &kala_contracts::model_json_limits())
        .expect("canonical");
    assert!(rendered.contains("\"memory_id\":\"m-1\""));
    assert!(rendered.contains("\"content\":\"alpha\""));
    assert!(rendered.contains("\"source\":\"source-1\""));
    assert!(rendered.contains("\"provenance_id\":null"));
    assert!(rendered.contains("\"metadata_json\":\"{}\""));
    assert!(rendered.contains("\"created_at\":\"2026-01-02T03:04:05Z\""));
    assert!(rendered.contains("\"expires_at\":null"));
}

#[test]
fn the_text_type_keeps_code_point_fidelity() {
    let content = Text::new("alpha");
    assert_eq!(content.to_string(), "alpha");
    assert_eq!(content.len(), 5);
    let record = record("m-1", "alpha", "source-1", RECORDED_AT, None);
    assert_eq!(record.content(), &Text::new("alpha"));
}

#[test]
fn a_repository_failure_is_reported_as_a_value_error() {
    let manager = manager(8);
    let error: MemoryError = manager.delete("../escape").expect_err("hostile identifier");
    assert_eq!(error.python_class_name(), "ValueError");
    assert!(error.as_repository().is_some());
}
