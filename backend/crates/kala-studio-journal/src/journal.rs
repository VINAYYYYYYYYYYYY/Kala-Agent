//! The Studio's own append-only, hash-chained session journal.
//!
//! The journal lives in a caller-supplied data directory and never touches
//! the core Kala database. The file is a single SQLite database in WAL mode
//! and every statement runs under one lock, so a server thread pool may use
//! one instance from several threads at once.
//!
//! A payload is stored as canonical JSON and is bounded in size; a page is
//! bounded in length; an artifact is bounded by the artifact store. Nothing
//! this type accepts can grow without a limit.
//!
//! The chain is walked once when the journal is opened and the verified head
//! is cached from then on, so an append verifies only the newest stored row
//! instead of the whole history.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use kala_artifacts::ContentAddressedArtifactStore;
use kala_json::{
    CanonicalValue, EventChainFields, GENESIS_PREVIOUS_DIGEST, Sha256Digest, chain_digest,
};
use kala_persistence::{
    canonicalize_object_value, parse_json_object, sha256_text, timestamp_to_iso, truncate_to_micros,
};
use parking_lot::Mutex;
use rusqlite::{Connection, OpenFlags, params};

use crate::error::StudioJournalError;
use crate::event::{JournalVerification, SessionEvent};
use crate::identifier::normalize_identifier;
use crate::path::{OWNER_ONLY_FILE_MODE, prepare_data_directory, set_owner_only_mode};
use crate::row::{CHAIN_COLUMNS, RawRow, row_is_self_consistent};

/// The name of the Studio journal database file.
pub const JOURNAL_FILE_NAME: &str = "studio-journal.sqlite3";
/// The name of the Studio artifact directory.
pub const ARTIFACT_DIRECTORY_NAME: &str = "artifacts";
/// The aggregate every Studio event is recorded under unless another is given.
pub const DEFAULT_AGGREGATE_ID: &str = "studio";
/// The largest canonical payload the journal accepts by default.
pub const DEFAULT_MAX_PAYLOAD_BYTES: usize = 256 * 1024;
/// The largest artifact the journal accepts by default.
pub const DEFAULT_MAX_ARTIFACT_BYTES: u64 = 64 * 1024 * 1024;
/// The largest page the journal returns by default.
pub const DEFAULT_MAX_PAGE_SIZE: i64 = 500;
/// The SQLite busy timeout the journal uses by default.
pub const DEFAULT_BUSY_TIMEOUT_MS: u32 = 5_000;

const CREATE_EVENTS_TABLE: &str = "
    CREATE TABLE IF NOT EXISTS studio_events (
        sequence INTEGER PRIMARY KEY,
        event_type TEXT NOT NULL,
        aggregate_id TEXT NOT NULL,
        workflow_id TEXT,
        run_id TEXT,
        payload_json TEXT NOT NULL,
        digest TEXT NOT NULL,
        occurred_at TEXT NOT NULL,
        previous_event_digest TEXT NOT NULL,
        event_digest TEXT NOT NULL
    )
    ";

const CREATE_EVENTS_INDEX: &str = "
    CREATE INDEX IF NOT EXISTS studio_events_type_sequence_idx
    ON studio_events(event_type, sequence)
    ";

const INSERT_EVENT: &str = "
    INSERT INTO studio_events(
        sequence, event_type, aggregate_id, workflow_id, run_id, payload_json,
        digest, occurred_at, previous_event_digest, event_digest
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    ";

/// The walk query, with the stored row count in the same snapshot.
///
/// The count travels in the same statement as the rows so the reported
/// `event_count` and the walked rows always describe one snapshot, exactly
/// like the Python module's single `fetchall()`.
const WALK_QUERY: &str = "
    SELECT sequence, event_type, aggregate_id, workflow_id, run_id,
           payload_json, digest, occurred_at, previous_event_digest, event_digest,
           (SELECT COUNT(*) FROM studio_events) AS studio_total_rows
    FROM studio_events
    ORDER BY sequence ASC
    ";

/// The bounds and timeouts the journal is opened with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalConfig {
    /// The largest canonical payload the journal accepts, in bytes.
    pub max_payload_bytes: usize,
    /// The largest artifact the journal accepts, in bytes.
    pub max_artifact_bytes: u64,
    /// The largest page the journal returns.
    pub max_page_size: i64,
    /// The SQLite busy timeout, in milliseconds.
    pub busy_timeout_ms: u32,
}

impl Default for JournalConfig {
    fn default() -> Self {
        Self {
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            max_artifact_bytes: DEFAULT_MAX_ARTIFACT_BYTES,
            max_page_size: DEFAULT_MAX_PAGE_SIZE,
            busy_timeout_ms: DEFAULT_BUSY_TIMEOUT_MS,
        }
    }
}

impl JournalConfig {
    fn validate(self) -> Result<Self, StudioJournalError> {
        if self.max_payload_bytes < 1 {
            return Err(StudioJournalError::NotPositive {
                field: "max_payload_bytes",
            });
        }
        if self.max_artifact_bytes < 1 {
            return Err(StudioJournalError::NotPositive {
                field: "max_artifact_bytes",
            });
        }
        if self.max_page_size < 1 {
            return Err(StudioJournalError::NotPositive {
                field: "max_page_size",
            });
        }
        Ok(self)
    }
}

/// The optional values of one append.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendOptions {
    /// The aggregate the event is recorded under.
    pub aggregate_id: String,
    /// An explicit occurrence time; the current UTC wall clock is used when it
    /// is absent.
    pub occurred_at: Option<DateTime<Utc>>,
}

impl Default for AppendOptions {
    fn default() -> Self {
        Self {
            aggregate_id: DEFAULT_AGGREGATE_ID.to_owned(),
            occurred_at: None,
        }
    }
}

impl AppendOptions {
    /// Set the aggregate the event is recorded under.
    pub fn with_aggregate_id(mut self, aggregate_id: impl Into<String>) -> Self {
        self.aggregate_id = aggregate_id.into();
        self
    }

    /// Set an explicit occurrence time.
    pub fn with_occurred_at(mut self, occurred_at: DateTime<Utc>) -> Self {
        self.occurred_at = Some(occurred_at);
        self
    }
}

/// Counters that describe how the journal spent its work.
///
/// The counters exist so the O(1) append guarantee can be observed instead of
/// assumed: a steady stream of appends must not walk the chain again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JournalDiagnostics {
    /// The number of full-chain walks the journal performed.
    pub chain_walks: u64,
    /// The number of appends that linked to the cached head.
    pub fast_path_appends: u64,
    /// The number of appends that had to walk the chain first.
    pub fallback_appends: u64,
}

/// The internal result of walking the stored chain.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ChainWalk {
    event_count: i64,
    verified_count: i64,
    head_digest: Option<String>,
    first_bad_sequence: Option<i64>,
}

/// The mutable state of one journal.
///
/// The state is kept apart from the connection so the connection can be
/// borrowed while the state is updated.
#[derive(Debug, Default)]
struct JournalState {
    head_sequence: i64,
    head_digest: Option<String>,
    first_bad_sequence: Option<i64>,
    closed: bool,
    diagnostics: JournalDiagnostics,
}

/// The journal internals.
struct Inner {
    connection: Option<Connection>,
    state: JournalState,
}

impl Inner {
    /// Return the open connection.
    fn connection(&self) -> Result<&Connection, StudioJournalError> {
        if self.state.closed {
            return Err(StudioJournalError::Closed);
        }
        self.connection.as_ref().ok_or(StudioJournalError::Closed)
    }

    /// Append one event inside a write transaction.
    #[allow(clippy::too_many_arguments)]
    fn append_in_transaction(
        &mut self,
        event_type: &str,
        aggregate_id: &str,
        payload_json: &str,
        payload_digest: kala_json::Sha256Digest,
        occurred_at: String,
    ) -> Result<SessionEvent, StudioJournalError> {
        let Inner { connection, state } = self;
        if state.closed {
            return Err(StudioJournalError::Closed);
        }
        let connection = connection.as_ref().ok_or(StudioJournalError::Closed)?;
        connection.execute_batch("BEGIN IMMEDIATE")?;
        match insert_event(
            state,
            connection,
            event_type,
            aggregate_id,
            payload_json,
            payload_digest,
            occurred_at,
        ) {
            Ok(event) => {
                connection.execute_batch("COMMIT")?;
                Ok(event)
            }
            Err(error) => {
                let _ = connection.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    /// Walk the stored chain and report the first event that fails.
    fn walk_chain(&mut self) -> Result<ChainWalk, StudioJournalError> {
        let Inner { connection, state } = self;
        if state.closed {
            return Err(StudioJournalError::Closed);
        }
        let connection = connection.as_ref().ok_or(StudioJournalError::Closed)?;
        walk_chain(state, connection)
    }
}

/// The Studio's own append-only, hash-chained session journal.
///
/// The journal lives in a caller-supplied data directory and never touches
/// the core Kala database. The file is a single SQLite database in WAL mode
/// and every statement runs under one lock, so a server thread pool may use
/// one instance from several threads at once.
///
/// A payload is stored as canonical JSON and is bounded in size; a page is
/// bounded in length; an artifact is bounded by the artifact store. Nothing
/// this type accepts can grow without a limit.
///
/// The chain is walked once when the journal is opened and the verified head
/// is cached from then on, so an append verifies only the newest stored row
/// instead of the whole history.
pub struct StudioSessionJournal {
    inner: Mutex<Inner>,
    artifacts: ContentAddressedArtifactStore,
    data_dir: PathBuf,
    database_path: PathBuf,
    max_payload_bytes: usize,
    max_page_size: i64,
}

impl std::fmt::Debug for StudioSessionJournal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StudioSessionJournal")
            .field("data_dir", &self.data_dir)
            .field("database_path", &self.database_path)
            .field("max_payload_bytes", &self.max_payload_bytes)
            .field("max_page_size", &self.max_page_size)
            .finish_non_exhaustive()
    }
}

impl StudioSessionJournal {
    /// Open the journal in `data_dir` with the default bounds.
    ///
    /// The directory is created if it does not exist. The whole chain is
    /// walked once, here, and the verified head is cached from then on.
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, StudioJournalError> {
        Self::open_with_config(data_dir, JournalConfig::default())
    }

    /// Open the journal in `data_dir` with explicit bounds.
    pub fn open_with_config(
        data_dir: impl AsRef<Path>,
        config: JournalConfig,
    ) -> Result<Self, StudioJournalError> {
        let config = config.validate()?;
        let data_dir = prepare_data_directory(data_dir.as_ref())?;
        let database_path = data_dir.join(JOURNAL_FILE_NAME);
        let artifacts = ContentAddressedArtifactStore::new(
            data_dir.join(ARTIFACT_DIRECTORY_NAME),
            config.max_artifact_bytes,
        )?;

        let connection = Connection::open_with_flags(
            &database_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        connection.busy_timeout(Duration::from_millis(u64::from(config.busy_timeout_ms)))?;

        let mut inner = Inner {
            connection: Some(connection),
            state: JournalState::default(),
        };
        let preparation = (|| -> Result<(), StudioJournalError> {
            // `PRAGMA journal_mode` returns a row, so it is read rather than
            // executed.
            let _mode: String =
                inner
                    .connection()?
                    .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
            inner
                .connection()?
                .execute_batch("PRAGMA synchronous = FULL")?;
            inner.connection()?.execute_batch(CREATE_EVENTS_TABLE)?;
            inner.connection()?.execute_batch(CREATE_EVENTS_INDEX)?;
            Ok(())
        })();
        if let Err(error) = preparation {
            inner.state.closed = true;
            inner.connection = None;
            return Err(error);
        }

        let journal = Self {
            inner: Mutex::new(inner),
            artifacts,
            data_dir,
            database_path,
            max_payload_bytes: config.max_payload_bytes,
            max_page_size: config.max_page_size,
        };
        {
            let mut inner = journal.inner.lock();
            let walk = inner.walk_chain()?;
            inner.state.apply_walk(&walk);
        }
        set_owner_only_mode(&journal.database_path, OWNER_ONLY_FILE_MODE);
        Ok(journal)
    }

    // --- identity ---

    /// Return the Studio data directory that holds the journal.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Return the path of the Studio journal database file.
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    /// Return the root of the Studio content-addressed artifact store.
    pub fn artifact_root(&self) -> PathBuf {
        self.data_dir.join(ARTIFACT_DIRECTORY_NAME)
    }

    /// Return the largest canonical payload the journal accepts.
    pub fn max_payload_bytes(&self) -> usize {
        self.max_payload_bytes
    }

    /// Return the largest page the journal will return.
    pub fn max_page_size(&self) -> i64 {
        self.max_page_size
    }

    /// Return the work counters of this journal.
    pub fn diagnostics(&self) -> JournalDiagnostics {
        self.inner.lock().state.diagnostics
    }

    // --- writing ---

    /// Append one event and return it with the digests it committed to.
    ///
    /// The payload is canonicalised before it is stored, so the bytes that
    /// are hashed are the bytes that are kept. The sequence is taken from the
    /// verified chain under the write lock.
    ///
    /// The write does not walk the whole chain. It re-reads only the newest
    /// stored row, re-verifies that row against its own recorded digests, and
    /// checks that it is still the cached head. If it is not -- the head was
    /// edited, or another instance appended to the same file -- the journal
    /// falls back to a full walk and refuses the append when that walk finds
    /// the chain broken instead of growing a second, inconsistent history.
    pub fn append(
        &self,
        event_type: &str,
        payload: &CanonicalValue,
    ) -> Result<SessionEvent, StudioJournalError> {
        self.append_with(event_type, payload, &AppendOptions::default())
    }

    /// Append one event with explicit optional values.
    pub fn append_with(
        &self,
        event_type: &str,
        payload: &CanonicalValue,
        options: &AppendOptions,
    ) -> Result<SessionEvent, StudioJournalError> {
        self.append_canonical(event_type, payload, options)
    }

    /// Append one event whose payload is given as JSON text.
    ///
    /// The text is parsed and canonicalised, exactly like the Python module
    /// canonicalises the mapping it is given. Text that is not a JSON object
    /// is refused.
    pub fn append_json(
        &self,
        event_type: &str,
        payload_json: &str,
    ) -> Result<SessionEvent, StudioJournalError> {
        self.append_json_with(event_type, payload_json, &AppendOptions::default())
    }

    /// Append one event whose payload is given as JSON text, with explicit
    /// optional values.
    pub fn append_json_with(
        &self,
        event_type: &str,
        payload_json: &str,
        options: &AppendOptions,
    ) -> Result<SessionEvent, StudioJournalError> {
        // Parsing the text is the caller's step in the Python module, so a
        // text that is not JSON at all is refused before the module's own
        // validation runs.
        let value = match parse_json_object(payload_json) {
            Ok(value) => value,
            Err(kala_persistence::PersistenceError::JsonObjectRequired) => {
                return Err(StudioJournalError::PayloadNotAMapping);
            }
            Err(kala_persistence::PersistenceError::CanonicalJson(_)) => {
                return Err(StudioJournalError::InvalidPayload);
            }
            Err(error) => return Err(error.into()),
        };
        self.append_canonical(event_type, &value, options)
    }

    /// Append one event, in the Python module's exact order of precedence.
    ///
    /// The Python module checks the payload type, then the event type, then
    /// the aggregate id, then the canonical form of the payload, and finally
    /// the size of the canonical payload. The order decides which error a
    /// caller sees when more than one rule is broken, so it is reproduced
    /// here rule for rule.
    fn append_canonical(
        &self,
        event_type: &str,
        payload: &CanonicalValue,
        options: &AppendOptions,
    ) -> Result<SessionEvent, StudioJournalError> {
        // The payload must be a mapping.
        if !matches!(payload, CanonicalValue::Object(_)) {
            return Err(StudioJournalError::PayloadNotAMapping);
        }
        // The event type and the aggregate id must be safe identifiers.
        let normalized_event_type = normalize_identifier(event_type, "event_type")?;
        let normalized_aggregate_id = normalize_identifier(&options.aggregate_id, "aggregate_id")?;
        // The payload must be canonicalisable.
        let payload_json = match canonicalize_object_value(payload) {
            Ok(canonical) => canonical,
            Err(kala_persistence::PersistenceError::JsonObjectRequired) => {
                return Err(StudioJournalError::PayloadNotAMapping);
            }
            Err(kala_persistence::PersistenceError::CanonicalJson(_)) => {
                return Err(StudioJournalError::InvalidPayload);
            }
            Err(error) => return Err(error.into()),
        };
        // The canonical payload must fit.
        if payload_json.len() > self.max_payload_bytes {
            return Err(StudioJournalError::PayloadTooLarge);
        }
        let payload_digest = sha256_text(&payload_json);
        let occurred_at = match options.occurred_at {
            Some(timestamp) => timestamp_to_iso(&timestamp)?,
            None => timestamp_to_iso(&truncate_to_micros(Utc::now()))?,
        };
        let mut inner = self.inner.lock();
        inner.append_in_transaction(
            &normalized_event_type,
            &normalized_aggregate_id,
            &payload_json,
            payload_digest,
            occurred_at,
        )
    }

    // --- reading ---

    /// Return at most `limit` events after `after_sequence`.
    ///
    /// The page is bounded twice: `limit` is capped at the journal's
    /// `max_page_size`, and the events come back in ascending sequence order.
    /// A page never fails because the chain is broken: the UI reads the
    /// history and asks [`Self::verify`] separately whether the history still
    /// verifies.
    pub fn page(
        &self,
        after_sequence: i64,
        limit: i64,
    ) -> Result<Vec<SessionEvent>, StudioJournalError> {
        if after_sequence < 0 {
            return Err(StudioJournalError::NegativeValue {
                field: "after_sequence",
            });
        }
        if limit < 1 {
            return Err(StudioJournalError::NotPositive { field: "limit" });
        }
        let effective_limit = limit.min(self.max_page_size);
        let inner = self.inner.lock();
        let connection = inner.connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {CHAIN_COLUMNS} FROM studio_events WHERE sequence > ? \
             ORDER BY sequence ASC LIMIT ?"
        ))?;
        let mut rows = statement.query(params![after_sequence, effective_limit])?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            events.push(event_from_row(&RawRow::from_row(row)?));
        }
        Ok(events)
    }

    /// Return the event with `sequence`, or `None` when it is absent.
    pub fn get(&self, sequence: i64) -> Result<Option<SessionEvent>, StudioJournalError> {
        if sequence < 0 {
            return Err(StudioJournalError::NegativeValue { field: "sequence" });
        }
        let inner = self.inner.lock();
        let connection = inner.connection()?;
        let mut statement = connection.prepare(&format!(
            "SELECT {CHAIN_COLUMNS} FROM studio_events WHERE sequence = ?"
        ))?;
        let mut rows = statement.query(params![sequence])?;
        match rows.next()? {
            Some(row) => Ok(Some(event_from_row(&RawRow::from_row(row)?))),
            None => Ok(None),
        }
    }

    /// Return the verified chain digest of the newest event, if any.
    ///
    /// Like the Python property, this walks the whole chain, so it reports the
    /// last *verified* digest rather than the cached head.
    pub fn head_digest(&self) -> Result<Option<String>, StudioJournalError> {
        Ok(self.verify()?.head_digest)
    }

    // --- verification ---

    /// Recompute the chain and report where it breaks, if it does.
    ///
    /// The whole journal is walked from the genesis event, so an edited,
    /// dropped, or reordered event is found by the first event whose recorded
    /// digest no longer matches its content or no longer links to its
    /// predecessor. The walk also refreshes the cached head, so an edit in the
    /// middle of the chain is noticed here and at the next open, and an edit
    /// of the head is noticed here and at the next append.
    pub fn verify(&self) -> Result<JournalVerification, StudioJournalError> {
        let mut inner = self.inner.lock();
        let walk = inner.walk_chain()?;
        inner.state.apply_walk(&walk);
        Ok(JournalVerification {
            ok: walk.first_bad_sequence.is_none(),
            event_count: walk.event_count,
            head_digest: walk.head_digest,
            first_bad_sequence: walk.first_bad_sequence,
        })
    }

    // --- artifacts ---

    /// Store `data` in the Studio artifact store and return its SHA-256.
    ///
    /// The caller records the returned digest in an `artifact.stored` event,
    /// which is what ties the blob to the design turn that produced it. The
    /// store is content-addressed, so storing the same bytes twice returns the
    /// same digest and creates no second object.
    pub fn put_artifact(&self, data: &[u8]) -> Result<String, StudioJournalError> {
        Ok(self.artifacts.put_bytes(data)?)
    }

    /// Return the bytes stored under `digest`.
    pub fn get_artifact(&self, digest: &str) -> Result<Vec<u8>, StudioJournalError> {
        Ok(self.artifacts.get_bytes(digest)?)
    }

    /// Return the bytes stored under `digest` decoded as UTF-8 text.
    pub fn get_artifact_text(&self, digest: &str) -> Result<String, StudioJournalError> {
        Ok(self.artifacts.get_text(digest)?)
    }

    /// Return whether an object with `digest` is stored.
    pub fn has_artifact(&self, digest: &str) -> Result<bool, StudioJournalError> {
        Ok(self.artifacts.contains(digest)?)
    }

    // --- lifecycle ---

    /// Close the journal database. Closing twice is harmless.
    pub fn close(&self) -> Result<(), StudioJournalError> {
        let mut inner = self.inner.lock();
        if inner.state.closed {
            return Ok(());
        }
        inner.state.closed = true;
        if let Some(connection) = inner.connection.take() {
            connection
                .close()
                .map_err(|(_, error)| StudioJournalError::Database(error))?;
        }
        Ok(())
    }
}

impl Drop for StudioSessionJournal {
    fn drop(&mut self) {
        let mut inner = self.inner.lock();
        inner.state.closed = true;
        inner.connection = None;
    }
}

impl JournalState {
    /// Cache the verified head and the first break a full walk found.
    fn apply_walk(&mut self, walk: &ChainWalk) {
        self.head_sequence = walk.verified_count;
        self.head_digest = walk.head_digest.clone();
        self.first_bad_sequence = walk.first_bad_sequence;
    }

    /// Return whether the newest stored row is the cached, verified head.
    ///
    /// The row is re-verified against the digests it records itself: its
    /// payload digest must match the bytes it stores, and its chain digest
    /// must match the chain digest recomputed over the covered fields it
    /// stores, including its stored previous digest.
    fn cached_head_matches(&self, connection: &Connection) -> Result<bool, StudioJournalError> {
        let mut statement = connection.prepare(&format!(
            "SELECT {CHAIN_COLUMNS} FROM studio_events ORDER BY sequence DESC LIMIT 1"
        ))?;
        let mut rows = statement.query([])?;
        let Some(row) = rows.next()? else {
            return Ok(self.head_sequence == 0 && self.head_digest.is_none());
        };
        let raw = RawRow::from_row(row)?;
        if raw.sequence != self.head_sequence {
            return Ok(false);
        }
        if Some(raw.event_digest.as_str()) != self.head_digest.as_deref() {
            return Ok(false);
        }
        Ok(row_is_self_consistent(&raw))
    }

    /// Return the sequence and the previous digest the new event links to.
    ///
    /// The fast path trusts the cached head when the newest stored row is
    /// still exactly that row, so a steady stream of appends costs one row
    /// read each. Anything else -- an edited head, an append by another
    /// instance on the same file -- falls back to a full walk, which is also
    /// the only way to notice an edit in the middle of the chain.
    fn resolve_append_head(
        &mut self,
        connection: &Connection,
    ) -> Result<(i64, String), StudioJournalError> {
        if let Some(sequence) = self.first_bad_sequence {
            return Err(StudioJournalError::Integrity { sequence });
        }
        if self.cached_head_matches(connection)? {
            self.diagnostics.fast_path_appends += 1;
            return match &self.head_digest {
                None => Ok((1, GENESIS_PREVIOUS_DIGEST.to_hex())),
                Some(digest) => Ok((self.head_sequence + 1, digest.clone())),
            };
        }
        self.diagnostics.fallback_appends += 1;
        let walk = walk_chain(self, connection)?;
        self.apply_walk(&walk);
        if let Some(sequence) = walk.first_bad_sequence {
            return Err(StudioJournalError::Integrity { sequence });
        }
        let previous_event_digest = match &walk.head_digest {
            None => GENESIS_PREVIOUS_DIGEST.to_hex(),
            Some(digest) => digest.clone(),
        };
        Ok((walk.verified_count + 1, previous_event_digest))
    }
}

/// Walk the stored chain and report the first event that fails.
fn walk_chain(
    state: &mut JournalState,
    connection: &Connection,
) -> Result<ChainWalk, StudioJournalError> {
    state.diagnostics.chain_walks += 1;
    let mut statement = connection.prepare(WALK_QUERY)?;
    let mut rows = statement.query([])?;
    let mut expected_sequence: i64 = 1;
    let mut previous_event_digest = GENESIS_PREVIOUS_DIGEST.to_hex();
    let mut verified_count: i64 = 0;
    let mut first_bad_sequence: Option<i64> = None;
    let mut event_count: i64 = 0;
    while let Some(row) = rows.next()? {
        event_count = row.get("studio_total_rows")?;
        let raw = RawRow::from_row(row)?;
        if first_bad_sequence.is_some() {
            // The Python walk stops at the first broken event; the stored row
            // count still covers every row, so the remaining rows are counted
            // but not judged.
            continue;
        }
        if raw.sequence != expected_sequence {
            first_bad_sequence = Some(expected_sequence);
            continue;
        }
        if raw.previous_event_digest != previous_event_digest {
            first_bad_sequence = Some(raw.sequence);
            continue;
        }
        if !row_is_self_consistent(&raw) {
            first_bad_sequence = Some(raw.sequence);
            continue;
        }
        previous_event_digest = raw.event_digest.clone();
        expected_sequence = raw.sequence + 1;
        verified_count += 1;
    }
    Ok(ChainWalk {
        event_count,
        verified_count,
        head_digest: if verified_count == 0 {
            None
        } else {
            Some(previous_event_digest)
        },
        first_bad_sequence,
    })
}

/// Insert one event inside an open write transaction.
#[allow(clippy::too_many_arguments)]
fn insert_event(
    state: &mut JournalState,
    connection: &Connection,
    event_type: &str,
    aggregate_id: &str,
    payload_json: &str,
    payload_digest: kala_json::Sha256Digest,
    occurred_at: String,
) -> Result<SessionEvent, StudioJournalError> {
    let (sequence, previous_event_digest) = state.resolve_append_head(connection)?;
    // The previous digest is either the genesis digest or a stored chain
    // digest, so it is always 64 lowercase hexadecimal digits.
    let previous_typed = Sha256Digest::from_hex(&previous_event_digest)
        .ok_or(StudioJournalError::Integrity { sequence })?;
    let fields = EventChainFields {
        sequence: u64::try_from(sequence)
            .map_err(|_| StudioJournalError::NegativeValue { field: "sequence" })?,
        event_type: event_type.to_owned(),
        aggregate_id: aggregate_id.to_owned(),
        workflow_id: None,
        run_id: None,
        occurred_at: occurred_at.clone(),
        payload_digest,
        previous_event_digest: previous_typed,
    };
    let event_digest = chain_digest(&fields);
    connection.execute(
        INSERT_EVENT,
        params![
            sequence,
            event_type,
            aggregate_id,
            Option::<String>::None,
            Option::<String>::None,
            payload_json,
            payload_digest.to_hex(),
            occurred_at,
            previous_event_digest,
            event_digest.to_hex(),
        ],
    )?;
    // The write lock is still held, so the cached head can move here without
    // another thread overwriting it.
    state.head_sequence = sequence;
    state.head_digest = Some(event_digest.to_hex());
    state.first_bad_sequence = None;
    Ok(SessionEvent {
        sequence,
        event_type: event_type.to_owned(),
        aggregate_id: aggregate_id.to_owned(),
        payload: decode_payload(payload_json),
        payload_json: payload_json.to_owned(),
        digest: payload_digest.to_hex(),
        occurred_at,
        previous_event_digest,
        event_digest: event_digest.to_hex(),
    })
}

/// Build one event from a stored row.
fn event_from_row(raw: &RawRow) -> SessionEvent {
    SessionEvent {
        sequence: raw.sequence,
        event_type: raw.event_type.clone(),
        aggregate_id: raw.aggregate_id.clone(),
        payload: decode_payload(&raw.payload_json),
        payload_json: raw.payload_json.clone(),
        digest: raw.digest.clone(),
        occurred_at: raw.occurred_at.clone(),
        previous_event_digest: raw.previous_event_digest.clone(),
        event_digest: raw.event_digest.clone(),
    }
}

/// Return the decoded payload, or `None` when the text is not an object.
fn decode_payload(payload_json: &str) -> Option<CanonicalValue> {
    parse_json_object(payload_json).ok()
}
