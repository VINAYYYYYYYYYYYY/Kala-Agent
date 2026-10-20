//! The result and entry values produced by the memory manager.

use kala_contracts::{ContractModel, Text};
use kala_json::CanonicalValue;
use kala_repositories::MemoryRecord;

use crate::status::MemoryStatus;

/// The outcome of [`crate::MemoryManager::try_add`].
///
/// This is the native counterpart of
/// `kala_agent.kernel.memory.MemoryAddResult`, a frozen dataclass with a
/// `status`, the `record` that was validated, and the optional `existing` and
/// `conflicting` records the outcome refers to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryAddResult {
    status: MemoryStatus,
    record: MemoryRecord,
    existing: Option<MemoryRecord>,
    conflicting: Option<MemoryRecord>,
}

impl MemoryAddResult {
    /// Build a result.
    pub fn new(
        status: MemoryStatus,
        record: MemoryRecord,
        existing: Option<MemoryRecord>,
        conflicting: Option<MemoryRecord>,
    ) -> Self {
        Self {
            status,
            record,
            existing,
            conflicting,
        }
    }

    /// Build an `added` result.
    pub fn added(record: MemoryRecord) -> Self {
        Self::new(MemoryStatus::Added, record, None, None)
    }

    /// Build a `duplicate` result.
    pub fn duplicate(record: MemoryRecord, existing: MemoryRecord) -> Self {
        Self::new(MemoryStatus::Duplicate, record, Some(existing), None)
    }

    /// Build a `conflict` result.
    pub fn conflict(record: MemoryRecord, conflicting: MemoryRecord) -> Self {
        Self::new(MemoryStatus::Conflict, record, None, Some(conflicting))
    }

    /// The outcome status.
    pub fn status(&self) -> MemoryStatus {
        self.status
    }

    /// The validated record.
    pub fn record(&self) -> &MemoryRecord {
        &self.record
    }

    /// The record that already held the content, for a duplicate.
    pub fn existing(&self) -> Option<&MemoryRecord> {
        self.existing.as_ref()
    }

    /// The record that already held the source with different content, for a
    /// conflict.
    pub fn conflicting(&self) -> Option<&MemoryRecord> {
        self.conflicting.as_ref()
    }

    /// The memory identifier of the validated record.
    pub fn memory_id(&self) -> &str {
        self.record.memory_id()
    }

    /// The content of the validated record.
    pub fn content(&self) -> &Text {
        self.record.content()
    }

    /// The source of the validated record.
    pub fn source(&self) -> &str {
        self.record.source()
    }

    /// The provenance identifier of the validated record.
    pub fn provenance_id(&self) -> Option<&str> {
        self.record.provenance_id()
    }

    /// Whether the outcome was accepted: added, durable or duplicate.
    pub fn accepted(&self) -> bool {
        self.status.is_accepted()
    }

    /// The Python `as_dict()` projection.
    ///
    /// The Python method returns
    /// `{"status": ..., "record": ..., "existing": ..., "conflicting": ...}`
    /// where the records are `model_dump(mode="json")` projections. The native
    /// projection is the same shape as a canonical JSON value, so the record
    /// objects carry the same field names and values.
    pub fn as_dict(&self) -> CanonicalValue {
        CanonicalValue::object([
            ("status", CanonicalValue::text(self.status.as_str())),
            (
                "record",
                self.record
                    .to_canonical_value()
                    .expect("a validated memory record always converts to a canonical value"),
            ),
            (
                "existing",
                self.existing
                    .as_ref()
                    .map_or(CanonicalValue::Null, |record| {
                        record.to_canonical_value().expect(
                            "a validated memory record always converts to a canonical value",
                        )
                    }),
            ),
            (
                "conflicting",
                self.conflicting
                    .as_ref()
                    .map_or(CanonicalValue::Null, |record| {
                        record.to_canonical_value().expect(
                            "a validated memory record always converts to a canonical value",
                        )
                    }),
            ),
        ])
    }
}

/// One working-memory entry.
///
/// This is the native counterpart of
/// `kala_agent.kernel.memory.MemoryEntry`, a frozen dataclass holding the
/// record and whether it is already durable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryEntry {
    record: MemoryRecord,
    durable: bool,
}

impl MemoryEntry {
    /// Build a working entry. The Python default for `durable` is `False`.
    pub fn new(record: MemoryRecord) -> Self {
        Self {
            record,
            durable: false,
        }
    }

    /// Build an entry with an explicit durability flag.
    pub fn with_durable(record: MemoryRecord, durable: bool) -> Self {
        Self { record, durable }
    }

    /// The stored record.
    pub fn record(&self) -> &MemoryRecord {
        &self.record
    }

    /// Whether the entry is already durable.
    pub fn durable(&self) -> bool {
        self.durable
    }
}
