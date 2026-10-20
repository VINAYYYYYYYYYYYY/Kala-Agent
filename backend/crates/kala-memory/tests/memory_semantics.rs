//! Native semantics tests ported from
//! `tests/unit/test_kernel_memory_planning.py` and the memory section of
//! `tests/unit/test_kernel.py`.
//!
//! Each test names the Python test it mirrors, so the two suites can be read
//! side by side.

use std::sync::Arc;
use std::thread;

use kala_contracts::{CanonicalJsonObject, ContractModel, Text, Timestamp};
use kala_memory::{
    AddOptions, CanonicalValue, GetOptions, ListOptions, ListWorkingOptions, MemoryAddResult,
    MemoryConflictError, MemoryEntry, MemoryError, MemoryManager, MemoryManagerConfig,
    MemoryRecord, MemoryRepository, MemoryStatus, PromoteOptions, PromoteTarget, is_expired,
    new_record, resolve_now, validated_record,
};
use kala_persistence::Database;

const RECORDED_AT: &str = "2026-01-02T03:04:05Z";

fn timestamp(value: &str) -> Timestamp {
    Timestamp::parse(value).expect("timestamp")
}

fn make_record(
    memory_id: &str,
    content: &str,
    source: &str,
    created_at: &str,
    expires_at: Option<&str>,
    provenance_id: Option<&str>,
) -> MemoryRecord {
    new_record(
        memory_id,
        content,
        source,
        provenance_id,
        CanonicalJsonObject::empty(),
        timestamp(created_at),
        expires_at.map(timestamp),
    )
    .expect("record")
}

fn simple(memory_id: &str, content: &str) -> MemoryRecord {
    make_record(memory_id, content, "source-1", RECORDED_AT, None, None)
}

/// A manager over a leaked in-memory database.
fn manager(max_working_records: usize) -> MemoryManager<'static> {
    let database: &'static Database = Box::leak(Box::new(Database::in_memory().expect("database")));
    MemoryManager::with_config(
        MemoryRepository::new(database),
        MemoryManagerConfig::new().with_max_working_records(max_working_records),
    )
    .expect("manager")
}

fn manager_with_provenance() -> MemoryManager<'static> {
    let database: &'static Database = Box::leak(Box::new(Database::in_memory().expect("database")));
    MemoryManager::with_config(
        MemoryRepository::new(database),
        MemoryManagerConfig::new().with_require_provenance_on_promotion(true),
    )
    .expect("manager")
}

// ---------------------------------------------------------------------------
// Module-level helpers.
// ---------------------------------------------------------------------------

#[test]
fn now_defaults_to_current_utc() {
    let generated = resolve_now(None);
    let now = resolve_now(Some(timestamp(RECORDED_AT)));
    assert!(generated.absolute_micros() > 0);
    assert_eq!(now, timestamp(RECORDED_AT));
}

#[test]
fn now_normalizes_offset_to_utc() {
    // The Python `_now` converts an offset value to UTC. The native timestamp
    // compares as an absolute instant, so the same equality holds.
    let offset = timestamp("2026-01-02T05:04:05+02:00");
    assert_eq!(resolve_now(Some(offset)), timestamp(RECORDED_AT));
}

#[test]
fn is_expired_compares_expiry() {
    let created = timestamp("2026-01-02T03:02:05Z");
    let now = timestamp(RECORDED_AT);
    assert!(!is_expired(&simple("m", "c"), &now));
    assert!(!is_expired(
        &make_record(
            "m",
            "c",
            "source-1",
            "2026-01-02T03:02:05Z",
            Some("2026-01-02T03:04:06Z"),
            None
        ),
        &now
    ));
    assert!(is_expired(
        &make_record(
            "m",
            "c",
            "source-1",
            "2026-01-02T03:02:05Z",
            Some(RECORDED_AT),
            None
        ),
        &now
    ));
    assert!(is_expired(
        &make_record(
            "m",
            "c",
            "source-1",
            "2026-01-02T03:02:05Z",
            Some("2026-01-02T03:04:04Z"),
            None
        ),
        &now
    ));
    let _ = created;
}

#[test]
fn validated_record_accepts_a_validated_record() {
    let record = simple("m", "c");
    let validated = validated_record(&record).expect("validated");
    assert_eq!(validated, record);
}

#[test]
fn validated_record_reports_contract_failures() {
    // The Python test mutates a record behind the model's back. The native
    // record type cannot be built invalid, so the equivalent failure is
    // observed at construction time.
    let error = new_record(
        "m",
        "c",
        "source-1",
        None,
        CanonicalJsonObject::empty(),
        timestamp(RECORDED_AT),
        Some(timestamp("2026-01-02T03:04:04Z")),
    )
    .expect_err("expiry must be later than creation");
    assert_eq!(error.python_class_name(), "ValidationError");
    assert_eq!(error.message(), "expires_at must be later than created_at");
}

// ---------------------------------------------------------------------------
// Result and entry values.
// ---------------------------------------------------------------------------

#[test]
fn memory_add_result_exposes_record_projection() {
    let record = make_record("m-1", "content", "source-1", RECORDED_AT, None, Some("p-1"));
    let result = MemoryAddResult::added(record);
    assert_eq!(result.memory_id(), "m-1");
    assert_eq!(result.content().to_string(), "content");
    assert_eq!(result.source(), "source-1");
    assert_eq!(result.provenance_id(), Some("p-1"));
    assert!(result.accepted());
    assert!(result.existing().is_none());
    assert!(result.conflicting().is_none());
    let payload = result.as_dict();
    assert_eq!(payload_status(&payload), "added");
    assert!(payload_is_null(&payload, "existing"));
    assert!(payload_is_null(&payload, "conflicting"));
}

#[test]
fn memory_add_result_rejects_conflict() {
    let record = simple("m-1", "content");
    let result = MemoryAddResult::conflict(record, simple("m-0", "other"));
    assert!(!result.accepted());
    let payload = result.as_dict();
    assert!(payload_is_null(&payload, "existing"));
    assert_eq!(payload_conflicting_content(&payload), "other");
}

#[test]
fn memory_add_result_duplicate_is_accepted() {
    let record = simple("m-1", "content");
    let result = MemoryAddResult::duplicate(record.clone(), record);
    assert!(result.accepted());
    assert_eq!(payload_existing_memory_id(&result.as_dict()), "m-1");
}

#[test]
fn memory_entry_defaults_to_working() {
    let entry = MemoryEntry::new(simple("m-1", "content"));
    assert!(!entry.durable());
    assert_eq!(entry.record().memory_id(), "m-1");
    let durable = MemoryEntry::with_durable(simple("m-2", "content"), true);
    assert!(durable.durable());
}

#[test]
fn memory_status_values_match_python() {
    assert_eq!(MemoryStatus::Added.as_str(), "added");
    assert_eq!(MemoryStatus::Durable.as_str(), "durable");
    assert_eq!(MemoryStatus::Duplicate.as_str(), "duplicate");
    assert_eq!(MemoryStatus::Conflict.as_str(), "conflict");
    assert!(MemoryStatus::Added.is_accepted());
    assert!(MemoryStatus::Durable.is_accepted());
    assert!(MemoryStatus::Duplicate.is_accepted());
    assert!(!MemoryStatus::Conflict.is_accepted());
}

#[test]
fn memory_conflict_error_carries_both_records() {
    let record = simple("m-1", "content");
    let conflicting = simple("m-0", "other");
    let error = MemoryConflictError::new(record.clone(), conflicting.clone());
    assert_eq!(error.record(), &record);
    assert_eq!(error.conflicting(), &conflicting);
    assert!(error.message().contains("source-1"));
    assert_eq!(
        error.message(),
        "memory source 'source-1' already contains different content"
    );
}

#[test]
fn memory_error_hierarchy_is_reported_by_class_name() {
    assert_eq!(
        MemoryError::Base("x".to_owned()).python_class_name(),
        "MemoryError"
    );
    assert_eq!(
        MemoryError::Validation("x".to_owned()).python_class_name(),
        "MemoryValidationError"
    );
    assert_eq!(
        MemoryError::NotFound("x".to_owned()).python_class_name(),
        "MemoryNotFoundError"
    );
    assert_eq!(
        MemoryError::Promotion("x".to_owned()).python_class_name(),
        "MemoryPromotionError"
    );
    assert_eq!(
        MemoryError::Type("x".to_owned()).python_class_name(),
        "TypeError"
    );
    assert_eq!(
        MemoryError::Value("x".to_owned()).python_class_name(),
        "ValueError"
    );
}

// ---------------------------------------------------------------------------
// Manager configuration.
// ---------------------------------------------------------------------------

#[test]
fn memory_manager_rejects_bad_configuration() {
    let database: &'static Database = Box::leak(Box::new(Database::in_memory().expect("database")));
    let error = MemoryManager::with_config(
        MemoryRepository::new(database),
        MemoryManagerConfig::new().with_max_working_records(0),
    )
    .expect_err("zero capacity");
    assert_eq!(error.python_class_name(), "ValueError");
    assert_eq!(error.message(), "max_working_records must be positive");
}

#[test]
fn memory_manager_exposes_configuration() {
    let manager = manager(7);
    assert_eq!(manager.max_working_records(), 7);
    assert!(!manager.require_provenance_on_promotion());
    assert!(manager.repository().get("absent").expect("read").is_none());
}

// ---------------------------------------------------------------------------
// Adding records.
// ---------------------------------------------------------------------------

#[test]
fn try_add_reports_duplicate_against_durable() {
    let manager = manager(1_000);
    let record = make_record("m-1", "shared", "source-1", RECORDED_AT, None, None);
    manager
        .promote(
            PromoteTarget::Record(&record),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    let other = make_record("m-2", "shared", "source-1", RECORDED_AT, None, None);
    let result = manager
        .try_add(&other, Some(timestamp(RECORDED_AT)))
        .expect("try_add");
    assert_eq!(result.status(), MemoryStatus::Duplicate);
    assert_eq!(result.existing().expect("existing").memory_id(), "m-1");
    assert!(result.accepted());
}

#[test]
fn try_add_reports_conflict_against_durable() {
    let manager = manager(1_000);
    let first = make_record("m-1", "first", "source-1", RECORDED_AT, None, None);
    manager
        .promote(
            PromoteTarget::Record(&first),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    let second = make_record("m-2", "second", "source-1", RECORDED_AT, None, None);
    let result = manager
        .try_add(&second, Some(timestamp(RECORDED_AT)))
        .expect("try_add");
    assert_eq!(result.status(), MemoryStatus::Conflict);
    assert_eq!(
        result.conflicting().expect("conflicting").memory_id(),
        "m-1"
    );
    assert!(!result.accepted());
}

#[test]
fn try_add_ignores_expired_durable_records() {
    let manager = manager(1_000);
    let first = make_record(
        "m-1",
        "first",
        "source-1",
        RECORDED_AT,
        Some("2026-01-02T03:05:05Z"),
        None,
    );
    manager
        .promote(
            PromoteTarget::Record(&first),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    let second = make_record("m-2", "second", "source-1", RECORDED_AT, None, None);
    let result = manager
        .try_add(&second, Some(timestamp("2026-01-02T03:06:05Z")))
        .expect("try_add");
    assert_eq!(result.status(), MemoryStatus::Added);
}

#[test]
fn try_add_reports_conflict_against_working() {
    let manager = manager(1_000);
    let first = make_record("m-1", "first", "source-1", RECORDED_AT, None, None);
    manager
        .add(
            &first,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let second = make_record("m-2", "second", "source-1", RECORDED_AT, None, None);
    let result = manager
        .try_add(&second, Some(timestamp(RECORDED_AT)))
        .expect("try_add");
    assert_eq!(result.status(), MemoryStatus::Conflict);
    assert_eq!(
        result.conflicting().expect("conflicting").memory_id(),
        "m-1"
    );
}

#[test]
fn try_add_reports_duplicate_against_working() {
    let manager = manager(1_000);
    let first = make_record("m-1", "shared", "source-1", RECORDED_AT, None, None);
    manager
        .add(
            &first,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let second = make_record("m-2", "shared", "source-1", RECORDED_AT, None, None);
    let result = manager
        .try_add(&second, Some(timestamp(RECORDED_AT)))
        .expect("try_add");
    assert_eq!(result.status(), MemoryStatus::Duplicate);
    assert_eq!(result.existing().expect("existing").memory_id(), "m-1");
}

#[test]
fn try_add_ignores_expired_working_records() {
    let manager = manager(1_000);
    let first = make_record(
        "m-1",
        "first",
        "source-1",
        RECORDED_AT,
        Some("2026-01-02T03:05:05Z"),
        None,
    );
    manager
        .add(
            &first,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let second = make_record("m-2", "second", "source-1", RECORDED_AT, None, None);
    let result = manager
        .try_add(&second, Some(timestamp("2026-01-02T03:06:05Z")))
        .expect("try_add");
    assert_eq!(result.status(), MemoryStatus::Added);
}

#[test]
fn try_add_detects_same_id_with_different_content() {
    let manager = manager(8);
    let first = make_record("m-1", "first", "source-a", RECORDED_AT, None, None);
    assert_eq!(
        manager
            .try_add(&first, Some(timestamp(RECORDED_AT)))
            .expect("try_add")
            .status(),
        MemoryStatus::Added
    );
    let conflicting = make_record("m-1", "different", "source-b", RECORDED_AT, None, None);
    let result = manager
        .try_add(&conflicting, Some(timestamp(RECORDED_AT)))
        .expect("try_add");
    assert_eq!(result.status(), MemoryStatus::Conflict);
    assert_eq!(
        result
            .conflicting()
            .expect("conflicting")
            .content()
            .to_string(),
        "first"
    );
}

#[test]
fn try_add_returns_existing_for_same_id_same_content() {
    let manager = manager(8);
    let record = make_record("m-1", "shared", "source-a", RECORDED_AT, None, None);
    manager
        .try_add(&record, Some(timestamp(RECORDED_AT)))
        .expect("try_add");
    let again = make_record("m-1", "shared", "source-b", RECORDED_AT, None, None);
    let result = manager
        .try_add(&again, Some(timestamp(RECORDED_AT)))
        .expect("try_add");
    assert_eq!(result.status(), MemoryStatus::Duplicate);
    assert_eq!(result.record(), result.existing().expect("existing"));
    assert_eq!(result.record().source(), "source-a");
}

#[test]
fn try_add_evicts_expired_records_at_capacity() {
    let manager = manager(2);
    let first = make_record(
        "m-1",
        "first",
        "source-1",
        RECORDED_AT,
        Some("2026-01-02T03:05:05Z"),
        None,
    );
    manager
        .add(
            &first,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let second = make_record("m-2", "second", "source-2", RECORDED_AT, None, None);
    manager
        .add(
            &second,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let third = make_record("m-3", "third", "source-3", RECORDED_AT, None, None);
    let later = "2026-01-02T03:06:05Z";
    let result = manager
        .try_add(&third, Some(timestamp(later)))
        .expect("try_add");
    assert_eq!(result.status(), MemoryStatus::Added);
    let listed = manager
        .list(
            ListOptions::new()
                .with_now(timestamp(later))
                .with_include_working(true)
                .with_durable_only(false),
        )
        .expect("list");
    let ids: Vec<&str> = listed.iter().map(|item| item.memory_id()).collect();
    assert_eq!(ids, ["m-2", "m-3"]);
}

#[test]
fn try_add_refuses_to_exceed_capacity() {
    let manager = manager(1);
    let first = make_record("m-1", "first", "source-1", RECORDED_AT, None, None);
    manager
        .add(
            &first,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let second = make_record("m-2", "second", "source-2", RECORDED_AT, None, None);
    let error = manager
        .try_add(&second, Some(timestamp(RECORDED_AT)))
        .expect_err("limit");
    assert_eq!(error.python_class_name(), "MemoryPromotionError");
    assert!(error.message().contains("limit exceeded"));
}

#[test]
fn add_raises_on_conflict() {
    let manager = manager(1_000);
    let first = make_record("m-1", "first", "source-1", RECORDED_AT, None, None);
    manager
        .add(
            &first,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let second = make_record("m-2", "second", "source-1", RECORDED_AT, None, None);
    let error = manager
        .add(
            &second,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect_err("conflict");
    assert_eq!(error.python_class_name(), "MemoryConflictError");
    assert!(error.as_conflict().is_some());
}

#[test]
fn add_promotes_when_requested() {
    let manager = manager(1_000);
    let record = simple("m-1", "content");
    manager
        .add(
            &record,
            AddOptions {
                durable: false,
                promote: true,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let durable = manager
        .list_durable(
            ListOptions::new()
                .with_now(timestamp(RECORDED_AT))
                .with_include_working(false)
                .with_durable_only(true),
        )
        .expect("list");
    assert_eq!(durable, vec![record]);
}

#[test]
fn add_promotes_when_durable_flag_set() {
    let manager = manager(1_000);
    let record = simple("m-1", "content");
    manager
        .add(
            &record,
            AddOptions {
                durable: true,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let durable = manager
        .list_durable(
            ListOptions::new()
                .with_now(timestamp(RECORDED_AT))
                .with_include_working(false)
                .with_durable_only(true),
        )
        .expect("list");
    assert_eq!(durable, vec![record]);
}

// ---------------------------------------------------------------------------
// Promotion.
// ---------------------------------------------------------------------------

#[test]
fn promote_by_id_from_working_memory() {
    let manager = manager(1_000);
    let record = simple("m-1", "content");
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
    let promoted = manager
        .promote(
            PromoteTarget::Id("m-1"),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    let durable = manager
        .list_durable(
            ListOptions::new()
                .with_now(timestamp(RECORDED_AT))
                .with_include_working(false)
                .with_durable_only(true),
        )
        .expect("list");
    assert_eq!(durable, vec![promoted]);
}

#[test]
fn promote_by_id_from_durable_repository() {
    let database: &'static Database = Box::leak(Box::new(Database::in_memory().expect("database")));
    let other = MemoryManager::new(MemoryRepository::new(database));
    let original = simple("m-1", "content");
    other
        .promote(
            PromoteTarget::Record(&original),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    // A second manager over the same repository has an empty working map, so
    // the promotion reads the record from durable storage.
    let manager = MemoryManager::new(MemoryRepository::new(database));
    let promoted = manager
        .promote(
            PromoteTarget::Id("m-1"),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    assert_eq!(promoted, original);
}

#[test]
fn promote_unknown_id_raises_not_found() {
    let manager = manager(1_000);
    let error = manager
        .promote(
            PromoteTarget::Id("absent"),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect_err("missing");
    assert_eq!(error.python_class_name(), "MemoryNotFoundError");
    assert_eq!(error.message(), "'absent'");
}

#[test]
fn promote_imports_working_record_not_in_memory() {
    let manager = manager(4);
    let record = make_record("m-1", "content", "source-a", RECORDED_AT, None, None);
    let promoted = manager
        .promote(
            PromoteTarget::Record(&record),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    assert_eq!(promoted, record);
    let durable = manager
        .list_durable(
            ListOptions::new()
                .with_now(timestamp(RECORDED_AT))
                .with_include_working(false)
                .with_durable_only(true),
        )
        .expect("list");
    assert_eq!(durable, vec![record]);
}

#[test]
fn promote_reports_conflict_against_durable() {
    let database: &'static Database = Box::leak(Box::new(Database::in_memory().expect("database")));
    let other = MemoryManager::new(MemoryRepository::new(database));
    let manager = MemoryManager::with_config(
        MemoryRepository::new(database),
        MemoryManagerConfig::new().with_max_working_records(4),
    )
    .expect("manager");
    // The Python test injects `m-2` straight into the private working map.
    // The native working map cannot be mutated from outside, so the same
    // state is built through the public API: `m-2` enters working memory
    // while no durable record of the source exists yet, and `m-1` is
    // promoted into durable storage by a second manager.
    manager
        .add(
            &make_record("m-2", "second", "source-1", RECORDED_AT, None, None),
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    other
        .promote(
            PromoteTarget::Record(&make_record(
                "m-1",
                "first",
                "source-1",
                RECORDED_AT,
                None,
                None,
            )),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    let error = manager
        .promote(
            PromoteTarget::Id("m-2"),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect_err("conflict");
    let conflict = error.as_conflict().expect("conflict");
    assert_eq!(conflict.conflicting().memory_id(), "m-1");
}

#[test]
fn promote_reports_conflict_against_other_working_records() {
    let manager = manager(4);
    let candidate = make_record(
        "m-1",
        "first",
        "source-1",
        RECORDED_AT,
        Some("2026-01-02T03:05:05Z"),
        None,
    );
    let rival = make_record("m-2", "second", "source-1", RECORDED_AT, None, None);
    manager
        .add(
            &candidate,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    // The Python test injects both working records directly, which the
    // native manager forbids. The same state is reachable publicly once the
    // first record has expired: an expired working record no longer
    // conflicts, so the second one enters the working map.
    let later = "2026-01-02T03:06:05Z";
    manager
        .add(
            &rival,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(later)),
            },
        )
        .expect("add");
    let error = manager
        .promote(
            PromoteTarget::Id("m-1"),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(later)),
            },
        )
        .expect_err("conflict");
    let conflict = error.as_conflict().expect("conflict");
    assert_eq!(conflict.conflicting().memory_id(), "m-2");
}

#[test]
fn promote_applies_provenance_override() {
    let manager = manager(1_000);
    let record = simple("m-1", "content");
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
    let promoted = manager
        .promote(
            PromoteTarget::Id("m-1"),
            PromoteOptions {
                provenance_id: Some("p-42".to_owned()),
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    assert_eq!(promoted.provenance_id(), Some("p-42"));
}

#[test]
fn promote_requires_provenance_when_configured() {
    let manager = manager_with_provenance();
    let record = simple("m-1", "content");
    let error = manager
        .promote(
            PromoteTarget::Record(&record),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect_err("provenance");
    assert_eq!(error.python_class_name(), "MemoryPromotionError");
    assert!(error.message().contains("requires provenance"));
}

// ---------------------------------------------------------------------------
// Reading.
// ---------------------------------------------------------------------------

#[test]
fn get_rejects_bad_identifiers() {
    let manager = manager(1_000);
    let error = manager
        .get(
            "   ",
            GetOptions {
                now: Some(timestamp(RECORDED_AT)),
                include_expired: false,
            },
        )
        .expect_err("blank");
    assert_eq!(error.python_class_name(), "ValueError");
    assert_eq!(error.message(), "memory_id must be nonempty");
}

#[test]
fn get_hides_expired_working_unless_requested() {
    let manager = manager(1_000);
    let record = make_record(
        "m-1",
        "content",
        "source-1",
        RECORDED_AT,
        Some("2026-01-02T03:05:05Z"),
        None,
    );
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
    assert_eq!(
        manager
            .get(
                "m-1",
                GetOptions {
                    now: Some(timestamp(later)),
                    include_expired: true,
                },
            )
            .expect("get"),
        Some(record)
    );
}

#[test]
fn get_falls_back_to_repository() {
    let database: &'static Database = Box::leak(Box::new(Database::in_memory().expect("database")));
    let manager = MemoryManager::new(MemoryRepository::new(database));
    let record = simple("m-1", "content");
    manager
        .promote(
            PromoteTarget::Record(&record),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    assert_eq!(
        manager
            .get(
                "m-1",
                GetOptions {
                    now: Some(timestamp(RECORDED_AT)),
                    include_expired: false,
                },
            )
            .expect("get"),
        Some(record)
    );
}

// ---------------------------------------------------------------------------
// Listing.
// ---------------------------------------------------------------------------

#[test]
fn list_validates_limit() {
    let manager = manager(1_000);
    let error = manager
        .list(ListOptions::new().with_limit(0))
        .expect_err("limit");
    assert_eq!(error.python_class_name(), "ValueError");
    assert_eq!(error.message(), "limit must be positive");
}

#[test]
fn list_filters_by_source_and_truncates() {
    let manager = manager(1_000);
    let first = make_record("m-1", "first", "source-1", RECORDED_AT, None, None);
    let second = make_record(
        "m-2",
        "second",
        "source-2",
        "2026-01-02T03:04:06Z",
        None,
        None,
    );
    for item in [&first, &second] {
        manager
            .add(
                item,
                AddOptions {
                    durable: false,
                    promote: false,
                    now: Some(timestamp(RECORDED_AT)),
                },
            )
            .expect("add");
    }
    let listed = |options: ListOptions| -> Vec<String> {
        manager
            .list(options)
            .expect("list")
            .iter()
            .map(|item| item.memory_id().to_owned())
            .collect()
    };
    let base = ListOptions::new()
        .with_now(timestamp(RECORDED_AT))
        .with_include_working(true)
        .with_durable_only(false);
    assert_eq!(listed(base.clone()), ["m-1", "m-2"]);
    assert_eq!(listed(base.clone().with_source("source-2")), ["m-2"]);
    assert_eq!(listed(base.with_limit(1)), ["m-1"]);
}

#[test]
fn list_skips_expired_and_supports_durable_only() {
    let manager = manager(1_000);
    let expiring = make_record(
        "m-1",
        "first",
        "source-1",
        RECORDED_AT,
        Some("2026-01-02T03:05:05Z"),
        None,
    );
    let durable = make_record("m-2", "second", "source-2", RECORDED_AT, None, None);
    manager
        .add(
            &expiring,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    manager
        .add(
            &durable,
            AddOptions {
                durable: false,
                promote: true,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let later = "2026-01-02T03:06:05Z";
    let listed = manager
        .list(
            ListOptions::new()
                .with_now(timestamp(later))
                .with_include_working(true)
                .with_durable_only(false),
        )
        .expect("list");
    let ids: Vec<&str> = listed.iter().map(|item| item.memory_id()).collect();
    assert_eq!(ids, ["m-2"]);
    let durable_only = manager
        .list_durable(
            ListOptions::new()
                .with_now(timestamp(RECORDED_AT))
                .with_include_working(false)
                .with_durable_only(true),
        )
        .expect("list");
    let ids: Vec<&str> = durable_only.iter().map(|item| item.memory_id()).collect();
    assert_eq!(ids, ["m-2"]);
}

#[test]
fn list_orders_by_creation_then_id() {
    let manager = manager(1_000);
    let later = make_record("m-2", "second", "source-2", RECORDED_AT, None, None);
    let earlier = make_record(
        "m-1",
        "first",
        "source-1",
        "2026-01-02T03:04:04Z",
        None,
        None,
    );
    for item in [&later, &earlier] {
        manager
            .add(
                item,
                AddOptions {
                    durable: false,
                    promote: false,
                    now: Some(timestamp(RECORDED_AT)),
                },
            )
            .expect("add");
    }
    let listed = manager
        .list(
            ListOptions::new()
                .with_now(timestamp(RECORDED_AT))
                .with_include_working(true)
                .with_durable_only(false),
        )
        .expect("list");
    let ids: Vec<&str> = listed.iter().map(|item| item.memory_id()).collect();
    assert_eq!(ids, ["m-1", "m-2"]);
}

#[test]
fn list_active_matches_list() {
    let manager = manager(1_000);
    let record = simple("m-1", "content");
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
    let options = ListOptions::new()
        .with_now(timestamp(RECORDED_AT))
        .with_include_working(true)
        .with_durable_only(false);
    assert_eq!(
        manager.list_active(options.clone()).expect("list"),
        manager.list(options).expect("list")
    );
    assert!(
        manager
            .list_active(
                ListOptions::new()
                    .with_now(timestamp(RECORDED_AT))
                    .with_include_working(false)
                    .with_durable_only(true),
            )
            .expect("list")
            .is_empty()
    );
}

#[test]
fn list_working_excludes_durable_and_expired() {
    let manager = manager(1_000);
    let durable = make_record("m-1", "first", "source-1", RECORDED_AT, None, None);
    let working = make_record("m-2", "second", "source-2", RECORDED_AT, None, None);
    let expiring = make_record(
        "m-3",
        "third",
        "source-3",
        RECORDED_AT,
        Some("2026-01-02T03:05:05Z"),
        None,
    );
    for (item, promote) in [(&durable, true), (&working, false), (&expiring, false)] {
        manager
            .add(
                item,
                AddOptions {
                    durable: false,
                    promote,
                    now: Some(timestamp(RECORDED_AT)),
                },
            )
            .expect("add");
    }
    let listed = |options: ListWorkingOptions| -> Vec<String> {
        manager
            .list_working(options)
            .expect("list")
            .iter()
            .map(|item| item.memory_id().to_owned())
            .collect()
    };
    let base = ListWorkingOptions::new().with_now(timestamp(RECORDED_AT));
    assert_eq!(listed(base.clone()), ["m-2", "m-3"]);
    assert_eq!(
        listed(base.clone().with_source("source-1")),
        Vec::<String>::new()
    );
    let later = ListWorkingOptions::new().with_now(timestamp("2026-01-02T03:06:05Z"));
    assert_eq!(listed(later), ["m-2"]);
}

#[test]
fn list_working_honours_limit() {
    let manager = manager(4);
    manager
        .add(
            &simple("m-1", "first"),
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    manager
        .add(
            &make_record("m-2", "second", "source-2", RECORDED_AT, None, None),
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let listed = manager
        .list_working(
            ListWorkingOptions::new()
                .with_now(timestamp(RECORDED_AT))
                .with_limit(1),
        )
        .expect("list");
    assert_eq!(listed.len(), 1);
}

// ---------------------------------------------------------------------------
// Conflicts, discarding and deleting.
// ---------------------------------------------------------------------------

#[test]
fn detect_conflicts_sorts_and_filters() {
    let manager = manager(8);
    let working_first = make_record("m-1", "first", "source-1", RECORDED_AT, None, None);
    let durable_second = make_record(
        "m-2",
        "second",
        "source-1",
        "2026-01-02T03:04:04Z",
        None,
        None,
    );
    manager
        .add(
            &working_first,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    manager
        .repository()
        .put(&durable_second)
        .expect("durable put");
    let conflicts = manager
        .detect_conflicts(
            &make_record("m-3", "third", "source-1", RECORDED_AT, None, None),
            Some(timestamp(RECORDED_AT)),
        )
        .expect("conflicts");
    let ids: Vec<&str> = conflicts.iter().map(|item| item.memory_id()).collect();
    assert_eq!(ids, ["m-2", "m-1"]);
    assert!(
        manager
            .detect_conflicts(
                &make_record("m-4", "other", "other", RECORDED_AT, None, None),
                Some(timestamp(RECORDED_AT)),
            )
            .expect("conflicts")
            .is_empty()
    );
}

#[test]
fn detect_conflicts_ignores_expired_entries() {
    let manager = manager(1_000);
    manager
        .add(
            &make_record(
                "m-1",
                "first",
                "source-1",
                RECORDED_AT,
                Some("2026-01-02T03:05:05Z"),
                None,
            ),
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    let later = "2026-01-02T03:06:05Z";
    assert!(
        manager
            .detect_conflicts(
                &make_record("m-2", "second", "source-1", RECORDED_AT, None, None),
                Some(timestamp(later)),
            )
            .expect("conflicts")
            .is_empty()
    );
}

#[test]
fn discard_working_only_removes_working() {
    let manager = manager(1_000);
    manager
        .add(
            &simple("m-1", "content"),
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    assert!(manager.discard_working("m-1"));
    assert!(!manager.discard_working("m-1"));
    manager
        .add(
            &simple("m-2", "content"),
            AddOptions {
                durable: false,
                promote: true,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    assert!(!manager.discard_working("m-2"));
    assert!(!manager.discard_working("absent"));
}

#[test]
fn delete_removes_working_and_durable() {
    let manager = manager(1_000);
    manager
        .add(
            &simple("m-1", "content"),
            AddOptions {
                durable: false,
                promote: true,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    assert!(manager.delete("m-1").expect("delete"));
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
            .is_none()
    );
    assert!(!manager.delete("absent").expect("delete"));
}

#[test]
fn active_durable_records_hides_expired() {
    let manager = manager(1_000);
    let record = make_record(
        "m-1",
        "content",
        "source-1",
        RECORDED_AT,
        Some("2026-01-02T03:05:05Z"),
        None,
    );
    manager
        .promote(
            PromoteTarget::Record(&record),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    let later = timestamp("2026-01-02T03:06:05Z");
    assert!(
        manager
            .active_durable_records(None, &later)
            .expect("durable")
            .is_empty()
    );
}

#[test]
fn memory_manager_serializes_concurrent_adds() {
    let manager = Arc::new(manager(200));
    let mut handles = Vec::new();
    for index in 0..10 {
        let manager = Arc::clone(&manager);
        handles.push(thread::spawn(move || {
            let record = make_record(
                &format!("m-{index}"),
                &format!("content-{index}"),
                &format!("source-{index}"),
                RECORDED_AT,
                None,
                None,
            );
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
        }));
    }
    for handle in handles {
        handle.join().expect("thread");
    }
    let listed = manager
        .list(
            ListOptions::new()
                .with_now(timestamp(RECORDED_AT))
                .with_include_working(true)
                .with_durable_only(false),
        )
        .expect("list");
    assert_eq!(listed.len(), 10);
}

#[test]
fn memory_manager_requires_explicit_promotion_and_detects_conflicts_and_expiry() {
    let database: &'static Database = Box::leak(Box::new(Database::in_memory().expect("database")));
    let repository = MemoryRepository::new(database);
    let manager = MemoryManager::new(repository);
    let expiring = make_record(
        "memory-1",
        "A bounded note.",
        "source-1",
        RECORDED_AT,
        Some("2026-01-02T03:05:05Z"),
        Some("provenance-memory-1"),
    );
    manager
        .add(
            &expiring,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("add");
    assert!(
        repository
            .list_with_options(
                &kala_repositories::MemoryListOptions::default().with_now(timestamp(RECORDED_AT))
            )
            .expect("durable")
            .is_empty()
    );
    assert_eq!(
        manager
            .list(
                ListOptions::new()
                    .with_now(timestamp(RECORDED_AT))
                    .with_include_working(true)
                    .with_durable_only(false),
            )
            .expect("list"),
        vec![expiring.clone()]
    );

    let promoted = manager
        .promote(
            PromoteTarget::Id("memory-1"),
            PromoteOptions {
                provenance_id: None,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect("promote");
    assert_eq!(promoted.provenance_id(), Some("provenance-memory-1"));
    assert_eq!(
        repository
            .get_with_options(
                "memory-1",
                &kala_repositories::MemoryGetOptions::default().with_now(timestamp(RECORDED_AT)),
            )
            .expect("read"),
        Some(promoted)
    );

    let error = manager
        .add(
            &make_record(
                "memory-2",
                "A contradictory note.",
                "source-1",
                RECORDED_AT,
                None,
                Some("provenance-memory-2"),
            ),
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(RECORDED_AT)),
            },
        )
        .expect_err("conflict");
    assert_eq!(error.python_class_name(), "MemoryConflictError");

    let later = "2026-01-02T03:05:05Z";
    assert!(
        manager
            .list(
                ListOptions::new()
                    .with_now(timestamp(later))
                    .with_include_working(true)
                    .with_durable_only(false),
            )
            .expect("list")
            .is_empty()
    );
    let replacement = make_record(
        "memory-3",
        "A replacement after expiry.",
        "source-1",
        RECORDED_AT,
        Some("2026-01-02T03:06:05Z"),
        Some("provenance-memory-3"),
    );
    manager
        .add(
            &replacement,
            AddOptions {
                durable: false,
                promote: false,
                now: Some(timestamp(later)),
            },
        )
        .expect("add");
    assert_eq!(
        manager
            .get(
                "memory-3",
                GetOptions {
                    now: Some(timestamp(later)),
                    include_expired: false,
                },
            )
            .expect("get"),
        Some(replacement)
    );
}

// ---------------------------------------------------------------------------
// Projection helpers.
// ---------------------------------------------------------------------------

fn payload_field<'a>(value: &'a CanonicalValue, key: &str) -> Option<&'a CanonicalValue> {
    match value {
        CanonicalValue::Object(object) => object.get(&kala_json::JsonText::new(key)),
        _ => None,
    }
}

fn payload_status(value: &CanonicalValue) -> String {
    payload_field(value, "status")
        .and_then(|value| value.as_text())
        .map(|value| value.to_string_lossy())
        .expect("status")
}

fn payload_is_null(value: &CanonicalValue, key: &str) -> bool {
    matches!(payload_field(value, key), Some(CanonicalValue::Null))
}

fn payload_conflicting_content(value: &CanonicalValue) -> String {
    payload_field(value, "conflicting")
        .and_then(|value| payload_field(value, "content"))
        .and_then(|value| value.as_text())
        .map(|value| value.to_string_lossy())
        .expect("conflicting content")
}

fn payload_existing_memory_id(value: &CanonicalValue) -> String {
    payload_field(value, "existing")
        .and_then(|value| payload_field(value, "memory_id"))
        .and_then(|value| value.as_text())
        .map(|value| value.to_string_lossy())
        .expect("existing memory id")
}

// Keep the unused-import checker honest about the re-exported contract
// helpers used by the projection assertions.
#[test]
fn contract_helpers_are_usable() {
    let record = simple("m-1", "content");
    let value = record.to_canonical_value().expect("canonical");
    assert!(matches!(value, CanonicalValue::Object(_)));
    assert_eq!(Text::new("x").to_string(), "x");
}
