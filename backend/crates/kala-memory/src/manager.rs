//! The native working/durable memory manager.
//!
//! This is the port of `kala_agent.kernel.memory.MemoryManager`. It keeps a
//! bounded, insertion-ordered *working* map in front of the durable
//! [`MemoryRepository`], enforces "one content per source" over both layers,
//! and expires records against an explicit instant.

use std::sync::{Mutex, MutexGuard, PoisonError};

use indexmap::IndexMap;
use kala_contracts::{Text, Timestamp};
use kala_repositories::{
    DEFAULT_LIST_LIMIT, MemoryGetOptions, MemoryListOptions, MemoryRecord, MemoryRepository,
};

use crate::error::{MemoryConflictError, MemoryError};
use crate::result::{MemoryAddResult, MemoryEntry};
use crate::status::MemoryStatus;

/// The default working-memory capacity, matching the Python default.
pub const DEFAULT_MAX_WORKING_RECORDS: usize = 1_000;

/// The number of durable records one source scan may consider.
///
/// The Python manager reads durable records through
/// `MemoryRepository.list(source=..., now=..., include_expired=False)`, which
/// keeps the repository's default `limit=1_000`. The limit is part of the
/// observable behaviour: a source with more than one thousand durable records
/// is only compared against the first thousand of them.
pub const DURABLE_RECORD_SCAN_LIMIT: i64 = DEFAULT_LIST_LIMIT;

/// The manager configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryManagerConfig {
    /// The maximum number of working records.
    pub max_working_records: usize,
    /// Whether a promotion requires the record to carry a provenance
    /// identifier.
    pub require_provenance_on_promotion: bool,
}

impl Default for MemoryManagerConfig {
    fn default() -> Self {
        Self {
            max_working_records: DEFAULT_MAX_WORKING_RECORDS,
            require_provenance_on_promotion: false,
        }
    }
}

impl MemoryManagerConfig {
    /// The default configuration.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the working-memory capacity.
    pub fn with_max_working_records(mut self, max_working_records: usize) -> Self {
        self.max_working_records = max_working_records;
        self
    }

    /// Require a provenance identifier on promotion.
    pub fn with_require_provenance_on_promotion(mut self, required: bool) -> Self {
        self.require_provenance_on_promotion = required;
        self
    }
}

/// Options for [`MemoryManager::add`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddOptions {
    /// Promote the record to durable storage.
    pub durable: bool,
    /// Promote the record to durable storage. Python's `promote` flag.
    pub promote: bool,
    /// The instant the attempt is evaluated at; the current time when absent.
    pub now: Option<Timestamp>,
}

/// Options for [`MemoryManager::promote`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PromoteOptions {
    /// Replace the record's provenance identifier.
    pub provenance_id: Option<String>,
    /// The instant the promotion is evaluated at; the current time when
    /// absent.
    pub now: Option<Timestamp>,
}

/// What to promote.
#[derive(Clone, Copy, Debug)]
pub enum PromoteTarget<'a> {
    /// Promote a record.
    Record(&'a MemoryRecord),
    /// Promote a working or durable record by identifier.
    Id(&'a str),
}

/// Options for [`MemoryManager::get`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GetOptions {
    /// The instant the lookup is evaluated at; the current time when absent.
    pub now: Option<Timestamp>,
    /// Whether an expired record is still returned.
    pub include_expired: bool,
}

/// Options for [`MemoryManager::list`], [`MemoryManager::list_active`] and
/// [`MemoryManager::list_durable`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListOptions {
    /// Restrict the result to one source.
    pub source: Option<String>,
    /// The instant the listing is evaluated at; the current time when absent.
    pub now: Option<Timestamp>,
    /// Whether working records are included.
    ///
    /// The Python source accepts this flag and never reads it: only
    /// `durable_only` decides whether the working map is consulted. The
    /// native port keeps the flag so callers can be ported one-to-one, and
    /// keeps the Python behaviour.
    pub include_working: bool,
    /// Whether only durable records are listed.
    pub durable_only: bool,
    /// The maximum number of records returned. Must be positive.
    pub limit: i64,
}

impl Default for ListOptions {
    fn default() -> Self {
        Self {
            source: None,
            now: None,
            include_working: true,
            durable_only: false,
            limit: DEFAULT_LIST_LIMIT,
        }
    }
}

impl ListOptions {
    /// The default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Restrict the result to one source.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Evaluate the listing at a fixed instant.
    pub fn with_now(mut self, now: Timestamp) -> Self {
        self.now = Some(now);
        self
    }

    /// Set the working-record flag.
    pub fn with_include_working(mut self, include_working: bool) -> Self {
        self.include_working = include_working;
        self
    }

    /// Set the durable-only flag.
    pub fn with_durable_only(mut self, durable_only: bool) -> Self {
        self.durable_only = durable_only;
        self
    }

    /// Set the result limit.
    pub fn with_limit(mut self, limit: i64) -> Self {
        self.limit = limit;
        self
    }
}

/// Options for [`MemoryManager::list_working`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListWorkingOptions {
    /// Restrict the result to one source.
    pub source: Option<String>,
    /// The instant the listing is evaluated at; the current time when absent.
    pub now: Option<Timestamp>,
    /// The slice bound applied to the working records.
    ///
    /// The Python `list_working` does not validate this value; it applies
    /// Python's slice semantics, so a negative bound drops that many records
    /// from the end and a bound of zero returns nothing. The native port
    /// keeps that behaviour.
    pub limit: i64,
}

impl Default for ListWorkingOptions {
    fn default() -> Self {
        Self {
            source: None,
            now: None,
            limit: DEFAULT_LIST_LIMIT,
        }
    }
}

impl ListWorkingOptions {
    /// The default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Restrict the result to one source.
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Evaluate the listing at a fixed instant.
    pub fn with_now(mut self, now: Timestamp) -> Self {
        self.now = Some(now);
        self
    }

    /// Set the slice bound.
    pub fn with_limit(mut self, limit: i64) -> Self {
        self.limit = limit;
        self
    }
}

/// The insertion-ordered working map.
#[derive(Debug, Default)]
struct WorkingState {
    entries: IndexMap<String, MemoryEntry>,
}

/// The native counterpart of `kala_agent.kernel.memory.MemoryManager`.
///
/// The manager owns a bounded working map and reads and writes the durable
/// [`MemoryRepository`]. Every public operation is serialized by one lock, so
/// the check-then-write sequences the Python `RLock` guards are atomic here
/// too.
#[derive(Debug)]
pub struct MemoryManager<'a> {
    repository: MemoryRepository<'a>,
    max_working_records: usize,
    require_provenance_on_promotion: bool,
    working: Mutex<WorkingState>,
}

impl<'a> MemoryManager<'a> {
    /// Build a manager with the default configuration.
    pub fn new(repository: MemoryRepository<'a>) -> Self {
        Self::with_config(repository, MemoryManagerConfig::default())
            .expect("the default configuration is valid")
    }

    /// Build a manager with an explicit configuration.
    ///
    /// A capacity below one is refused with the Python message
    /// `"max_working_records must be positive"`.
    pub fn with_config(
        repository: MemoryRepository<'a>,
        config: MemoryManagerConfig,
    ) -> Result<Self, MemoryError> {
        if config.max_working_records < 1 {
            return Err(MemoryError::Value(
                "max_working_records must be positive".to_owned(),
            ));
        }
        Ok(Self {
            repository,
            max_working_records: config.max_working_records,
            require_provenance_on_promotion: config.require_provenance_on_promotion,
            working: Mutex::new(WorkingState::default()),
        })
    }

    /// The durable repository.
    pub fn repository(&self) -> &MemoryRepository<'a> {
        &self.repository
    }

    /// The working-memory capacity.
    pub fn max_working_records(&self) -> usize {
        self.max_working_records
    }

    /// Whether a promotion requires a provenance identifier.
    pub fn require_provenance_on_promotion(&self) -> bool {
        self.require_provenance_on_promotion
    }

    /// Add a record to working memory, optionally promoting it.
    ///
    /// The Python `add`/`add_record` pair. A conflicting source raises
    /// [`MemoryError::Conflict`]; otherwise the record is returned, or
    /// promoted when `durable` or `promote` is set.
    pub fn add(
        &self,
        record: &MemoryRecord,
        options: AddOptions,
    ) -> Result<MemoryRecord, MemoryError> {
        let result = self.try_add(record, options.now.clone())?;
        if result.status() == MemoryStatus::Conflict {
            let conflicting = result
                .conflicting()
                .cloned()
                .expect("a conflict result always carries the conflicting record");
            return Err(MemoryError::from(MemoryConflictError::new(
                result.record().clone(),
                conflicting,
            )));
        }
        let target = result
            .existing()
            .cloned()
            .unwrap_or_else(|| result.record().clone());
        if options.durable || options.promote {
            return self.promote(
                PromoteTarget::Record(&target),
                PromoteOptions {
                    provenance_id: None,
                    now: options.now,
                },
            );
        }
        Ok(target)
    }

    /// Add a record to working memory and report the outcome.
    ///
    /// The Python `try_add`.
    pub fn try_add(
        &self,
        record: &MemoryRecord,
        now: Option<Timestamp>,
    ) -> Result<MemoryAddResult, MemoryError> {
        let mut state = self.lock_working();
        self.try_add_locked(&mut state, record, now)
    }

    /// Promote a record to durable storage.
    ///
    /// The Python `promote`/`promote_record` pair.
    pub fn promote(
        &self,
        target: PromoteTarget<'_>,
        options: PromoteOptions,
    ) -> Result<MemoryRecord, MemoryError> {
        let current_time = resolve_now(options.now.clone());
        let mut state = self.lock_working();
        self.promote_locked(&mut state, target, options, current_time)
    }

    /// Read one record from working memory or the durable repository.
    ///
    /// The Python `get`.
    pub fn get(
        &self,
        memory_id: &str,
        options: GetOptions,
    ) -> Result<Option<MemoryRecord>, MemoryError> {
        if Text::new(memory_id).python_strip().is_empty() {
            return Err(MemoryError::Value("memory_id must be nonempty".to_owned()));
        }
        let current_time = resolve_now(options.now);
        let state = self.lock_working();
        if let Some(entry) = state.entries.get(memory_id) {
            if options.include_expired || !is_expired(entry.record(), &current_time) {
                return Ok(Some(entry.record().clone()));
            }
        }
        let repository_options = MemoryGetOptions::default()
            .with_now(current_time)
            .with_include_expired(options.include_expired);
        Ok(self
            .repository
            .get_with_options(memory_id, &repository_options)?)
    }

    /// List the active records.
    ///
    /// The Python `list`.
    pub fn list(&self, options: ListOptions) -> Result<Vec<MemoryRecord>, MemoryError> {
        let current_time = resolve_now(options.now);
        if options.limit < 1 {
            return Err(MemoryError::Value("limit must be positive".to_owned()));
        }
        let state = self.lock_working();
        let mut records: IndexMap<String, MemoryRecord> = IndexMap::new();
        if !options.durable_only {
            for entry in state.entries.values() {
                if let Some(source) = &options.source {
                    if entry.record().source() != source {
                        continue;
                    }
                }
                if is_expired(entry.record(), &current_time) {
                    continue;
                }
                records.insert(
                    entry.record().memory_id().to_owned(),
                    entry.record().clone(),
                );
            }
        }
        for record in self.active_durable_records(options.source.as_deref(), &current_time)? {
            records.insert(record.memory_id().to_owned(), record);
        }
        let mut ordered: Vec<MemoryRecord> = records.into_values().collect();
        ordered.sort_by(|left, right| {
            (left.created_at(), left.memory_id()).cmp(&(right.created_at(), right.memory_id()))
        });
        let end = usize::try_from(options.limit).unwrap_or(usize::MAX);
        Ok(ordered.into_iter().take(end).collect())
    }

    /// List the active records.
    ///
    /// The Python `list_active`, which forwards to `list`.
    pub fn list_active(&self, options: ListOptions) -> Result<Vec<MemoryRecord>, MemoryError> {
        self.list(options)
    }

    /// List the durable records.
    ///
    /// The Python `list_durable`, which forwards to `list` with
    /// `durable_only=True`.
    pub fn list_durable(&self, options: ListOptions) -> Result<Vec<MemoryRecord>, MemoryError> {
        self.list(options.with_include_working(false).with_durable_only(true))
    }

    /// List the working records that are not durable.
    ///
    /// The Python `list_working`.
    pub fn list_working(
        &self,
        options: ListWorkingOptions,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        let current_time = resolve_now(options.now);
        let state = self.lock_working();
        let mut records: Vec<MemoryRecord> = state
            .entries
            .values()
            .filter(|entry| !entry.durable())
            .filter(|entry| match &options.source {
                Some(source) => entry.record().source() == source,
                None => true,
            })
            .filter(|entry| !is_expired(entry.record(), &current_time))
            .map(|entry| entry.record().clone())
            .collect();
        records.sort_by(|left, right| {
            (left.created_at(), left.memory_id()).cmp(&(right.created_at(), right.memory_id()))
        });
        let end = python_slice_end(records.len(), options.limit);
        Ok(records.into_iter().take(end).collect())
    }

    /// List the records that hold the same source with different content.
    ///
    /// The Python `detect_conflicts`.
    pub fn detect_conflicts(
        &self,
        record: &MemoryRecord,
        now: Option<Timestamp>,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        let normalized = validated_record(record)?;
        let current_time = resolve_now(now);
        let state = self.lock_working();
        let mut candidates =
            self.active_durable_records(Some(normalized.source()), &current_time)?;
        for entry in state.entries.values() {
            if entry.record().source() == normalized.source()
                && !is_expired(entry.record(), &current_time)
            {
                candidates.push(entry.record().clone());
            }
        }
        let mut unique: IndexMap<String, MemoryRecord> = IndexMap::new();
        for candidate in candidates {
            if candidate.content() != normalized.content() {
                unique.insert(candidate.memory_id().to_owned(), candidate);
            }
        }
        let mut ordered: Vec<MemoryRecord> = unique.into_values().collect();
        ordered.sort_by(|left, right| {
            (left.created_at(), left.memory_id()).cmp(&(right.created_at(), right.memory_id()))
        });
        Ok(ordered)
    }

    /// Drop one working record that is not durable.
    ///
    /// The Python `discard_working`.
    pub fn discard_working(&self, memory_id: &str) -> bool {
        let mut state = self.lock_working();
        if state
            .entries
            .get(memory_id)
            .is_some_and(|entry| entry.durable())
        {
            return false;
        }
        state.entries.shift_remove(memory_id).is_some()
    }

    /// Delete one record from working memory and from the durable repository.
    ///
    /// The Python `delete`.
    pub fn delete(&self, memory_id: &str) -> Result<bool, MemoryError> {
        let mut state = self.lock_working();
        state.entries.shift_remove(memory_id);
        Ok(self.repository.delete(memory_id)?)
    }

    /// The active durable records of one source, or of every source.
    ///
    /// The Python `_active_durable_records`.
    pub fn active_durable_records(
        &self,
        source: Option<&str>,
        now: &Timestamp,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        let mut options = MemoryListOptions {
            now: Some(now.clone()),
            include_expired: false,
            limit: DURABLE_RECORD_SCAN_LIMIT,
            ..MemoryListOptions::default()
        };
        if let Some(source) = source {
            options = options.with_source(source);
        }
        let records = self.repository.list_with_options(&options)?;
        Ok(records
            .into_iter()
            .filter(|record| !is_expired(record, now))
            .collect())
    }

    fn try_add_locked(
        &self,
        state: &mut WorkingState,
        record: &MemoryRecord,
        now: Option<Timestamp>,
    ) -> Result<MemoryAddResult, MemoryError> {
        let normalized = validated_record(record)?;
        let current_time = resolve_now(now);

        let durable_records =
            self.active_durable_records(Some(normalized.source()), &current_time)?;
        if let Some(conflicting) = durable_records
            .iter()
            .find(|item| item.content() != normalized.content())
        {
            return Ok(MemoryAddResult::conflict(normalized, conflicting.clone()));
        }
        if let Some(duplicate) = durable_records
            .iter()
            .find(|item| item.content() == normalized.content())
        {
            return Ok(MemoryAddResult::duplicate(normalized, duplicate.clone()));
        }

        let working_conflict = state.entries.values().find(|entry| {
            entry.record().source() == normalized.source()
                && !is_expired(entry.record(), &current_time)
                && entry.record().content() != normalized.content()
        });
        if let Some(conflict) = working_conflict {
            return Ok(MemoryAddResult::conflict(
                normalized,
                conflict.record().clone(),
            ));
        }
        let working_duplicate = state.entries.values().find(|entry| {
            entry.record().source() == normalized.source()
                && !is_expired(entry.record(), &current_time)
                && entry.record().content() == normalized.content()
        });
        if let Some(duplicate) = working_duplicate {
            return Ok(MemoryAddResult::duplicate(
                normalized,
                duplicate.record().clone(),
            ));
        }

        if let Some(existing) = state.entries.get(normalized.memory_id()) {
            if existing.record().content() != normalized.content() {
                return Ok(MemoryAddResult::conflict(
                    normalized,
                    existing.record().clone(),
                ));
            }
            return Ok(MemoryAddResult::duplicate(
                existing.record().clone(),
                existing.record().clone(),
            ));
        }

        if state.entries.len() >= self.max_working_records {
            let expired: Vec<String> = state
                .entries
                .iter()
                .filter(|(_, entry)| is_expired(entry.record(), &current_time))
                .map(|(memory_id, _)| memory_id.clone())
                .collect();
            for memory_id in expired {
                state.entries.shift_remove(&memory_id);
            }
        }
        if state.entries.len() >= self.max_working_records {
            return Err(MemoryError::Promotion(
                "working memory limit exceeded".to_owned(),
            ));
        }
        state.entries.insert(
            normalized.memory_id().to_owned(),
            MemoryEntry::new(normalized.clone()),
        );
        Ok(MemoryAddResult::added(normalized))
    }

    fn promote_locked(
        &self,
        state: &mut WorkingState,
        target: PromoteTarget<'_>,
        options: PromoteOptions,
        current_time: Timestamp,
    ) -> Result<MemoryRecord, MemoryError> {
        let mut missing_key: Option<String> = None;
        let resolved: Option<MemoryRecord> = match target {
            PromoteTarget::Record(record) => {
                let validated = validated_record(record)?;
                if !state.entries.contains_key(validated.memory_id()) {
                    let result =
                        self.try_add_locked(state, &validated, Some(current_time.clone()))?;
                    if result.status() == MemoryStatus::Conflict {
                        let conflicting = result
                            .conflicting()
                            .cloned()
                            .unwrap_or_else(|| validated.clone());
                        return Err(MemoryError::from(MemoryConflictError::new(
                            validated,
                            conflicting,
                        )));
                    }
                    Some(validated)
                } else {
                    Some(
                        state
                            .entries
                            .get(validated.memory_id())
                            .map(|entry| entry.record().clone())
                            .unwrap_or(validated),
                    )
                }
            }
            PromoteTarget::Id(memory_id) => {
                let working = state
                    .entries
                    .get(memory_id)
                    .map(|entry| entry.record().clone());
                match working {
                    Some(record) => Some(record),
                    None => {
                        let repository_options = MemoryGetOptions::default()
                            .with_now(current_time.clone())
                            .with_include_expired(true);
                        match self
                            .repository
                            .get_with_options(memory_id, &repository_options)?
                        {
                            Some(record) => Some(record),
                            None => {
                                missing_key = Some(memory_id.to_owned());
                                None
                            }
                        }
                    }
                }
            }
        };

        let mut candidate = match resolved {
            Some(candidate) => candidate,
            None => {
                return Err(MemoryError::NotFound(missing_key.unwrap_or_default()));
            }
        };

        if let Some(provenance_id) = &options.provenance_id {
            candidate = MemoryRecord::new(
                candidate.memory_id(),
                candidate.content().clone(),
                candidate.source(),
                Some(provenance_id.as_str()),
                candidate.metadata_json().clone(),
                candidate.created_at().clone(),
                candidate.expires_at().cloned(),
            )
            .map_err(MemoryError::Contract)?;
        }

        if self.require_provenance_on_promotion && candidate.provenance_id().is_none() {
            return Err(MemoryError::Promotion(
                "durable memory requires provenance".to_owned(),
            ));
        }

        let durable_records =
            self.active_durable_records(Some(candidate.source()), &current_time)?;
        if let Some(conflicting) = durable_records
            .iter()
            .find(|item| item.content() != candidate.content())
        {
            return Err(MemoryError::from(MemoryConflictError::new(
                candidate,
                conflicting.clone(),
            )));
        }
        let working_conflict = state.entries.values().find(|entry| {
            entry.record().memory_id() != candidate.memory_id()
                && entry.record().source() == candidate.source()
                && !is_expired(entry.record(), &current_time)
                && entry.record().content() != candidate.content()
        });
        if let Some(conflict) = working_conflict {
            return Err(MemoryError::from(MemoryConflictError::new(
                candidate,
                conflict.record().clone(),
            )));
        }

        self.repository.put(&candidate)?;
        state.entries.insert(
            candidate.memory_id().to_owned(),
            MemoryEntry::with_durable(candidate.clone(), true),
        );
        Ok(candidate)
    }

    fn lock_working(&self) -> MutexGuard<'_, WorkingState> {
        self.working.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Re-validate one record the way the Python `_validated_record` does.
///
/// The Python source dumps the record and validates it again, so a record
/// that was mutated behind the model's back is refused with
/// `MemoryValidationError("record failed validation")`. The native
/// [`MemoryRecord`] cannot be built invalid, so the re-validation is
/// idempotent; the failure path stays reachable for callers that assemble
/// records from untrusted parts.
pub fn validated_record(record: &MemoryRecord) -> Result<MemoryRecord, MemoryError> {
    MemoryRecord::new(
        record.memory_id(),
        record.content().clone(),
        record.source(),
        record.provenance_id(),
        record.metadata_json().clone(),
        record.created_at().clone(),
        record.expires_at().cloned(),
    )
    .map_err(|_| MemoryError::Validation("record failed validation".to_owned()))
}

/// Build a validated [`MemoryRecord`] with the memory module's error type.
///
/// This is the native entry point for the Python `MemoryRecord(...)`
/// constructor followed by `_validated_record`. A contract failure is
/// reported as [`MemoryError::Contract`], the native counterpart of Pydantic's
/// `ValidationError`.
pub fn new_record(
    memory_id: &str,
    content: &str,
    source: &str,
    provenance_id: Option<&str>,
    metadata_json: kala_contracts::CanonicalJsonObject,
    created_at: Timestamp,
    expires_at: Option<Timestamp>,
) -> Result<MemoryRecord, MemoryError> {
    MemoryRecord::new(
        memory_id,
        Text::new(content),
        source,
        provenance_id,
        metadata_json,
        created_at,
        expires_at,
    )
    .map_err(MemoryError::Contract)
}

/// Whether a record is expired at the given instant.
///
/// The Python `_is_expired`: a record with an expiry is expired once the
/// expiry instant is reached, so `expires_at == now` is expired.
pub fn is_expired(record: &MemoryRecord, now: &Timestamp) -> bool {
    record
        .expires_at()
        .is_some_and(|expires_at| expires_at <= now)
}

/// The current instant, in Python's microsecond precision.
fn current_timestamp() -> Timestamp {
    let now = chrono::Utc::now();
    let micros =
        i128::from(now.timestamp()) * 1_000_000 + i128::from(now.timestamp_subsec_micros());
    Timestamp::from_absolute_micros(micros)
        .expect("the current instant is inside the Python datetime range")
}

/// Resolve the `now` argument the way the Python `_now` does.
///
/// A supplied instant is used as it is; an absent one becomes the current
/// time. The Python source raises `ValueError("now must include a timezone")`
/// for a naive value, which the native aware [`Timestamp`] type makes
/// impossible.
pub fn resolve_now(now: Option<Timestamp>) -> Timestamp {
    now.unwrap_or_else(current_timestamp)
}

/// Python's `records[:limit]` slice bound.
fn python_slice_end(len: usize, limit: i64) -> usize {
    if limit >= 0 {
        let bound = usize::try_from(limit).unwrap_or(usize::MAX);
        bound.min(len)
    } else {
        let bound = usize::try_from(limit.unsigned_abs()).unwrap_or(usize::MAX);
        len.saturating_sub(bound)
    }
}
