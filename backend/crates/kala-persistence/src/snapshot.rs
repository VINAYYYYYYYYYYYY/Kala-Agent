//! Workflow snapshots.
//!
//! A snapshot is canonical object JSON plus its SHA-256, written in a
//! serialized transaction and verified on every `get`/`latest`. This mirrors
//! `kala_agent.persistence.database.SnapshotStore`.

use chrono::{DateTime, Utc};
use kala_json::{CanonicalValue, Sha256Digest};
use rusqlite::{Row, params};

use crate::canonical::{
    canonicalize_json_object, canonicalize_object_value, is_lowercase_sha256_hex,
    parse_json_object, sha256_text,
};
use crate::database::Database;
use crate::error::PersistenceError;
use crate::journal::validate_identifier;
use crate::time::{timestamp_from_iso, timestamp_to_iso, truncate_to_micros};

const SNAPSHOT_COLUMNS: &str = "sequence, snapshot_id, workflow_id, state_json, digest, created_at";

/// One verified workflow snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkflowSnapshot {
    /// Auto-incrementing snapshot sequence.
    pub sequence: i64,
    /// Snapshot identifier.
    pub snapshot_id: String,
    /// Workflow identifier.
    pub workflow_id: String,
    /// Canonical state JSON exactly as persisted.
    pub state_json: String,
    /// SHA-256 of the persisted state bytes.
    pub digest: Sha256Digest,
    /// Creation timestamp in UTC.
    pub created_at: DateTime<Utc>,
}

impl WorkflowSnapshot {
    /// Parse the persisted state, which verification already canonicalized.
    pub fn state(&self) -> Result<CanonicalValue, PersistenceError> {
        parse_json_object(&self.state_json)
    }
}

/// Snapshot reads and writes for one database.
pub struct SnapshotStore<'a> {
    database: &'a Database,
}

impl<'a> SnapshotStore<'a> {
    /// Bind a store to a database.
    pub fn new(database: &'a Database) -> Self {
        Self { database }
    }

    /// Save a snapshot from canonical JSON text.
    pub fn save(
        &self,
        snapshot_id: &str,
        workflow_id: &str,
        state_json: &str,
        created_at: Option<DateTime<Utc>>,
    ) -> Result<WorkflowSnapshot, PersistenceError> {
        let canonical = canonicalize_json_object(state_json)?;
        self.save_canonical(snapshot_id, workflow_id, canonical, created_at)
    }

    /// Save a snapshot from a typed canonical value.
    pub fn save_value(
        &self,
        snapshot_id: &str,
        workflow_id: &str,
        state: &CanonicalValue,
        created_at: Option<DateTime<Utc>>,
    ) -> Result<WorkflowSnapshot, PersistenceError> {
        let canonical = canonicalize_object_value(state)?;
        self.save_canonical(snapshot_id, workflow_id, canonical, created_at)
    }

    fn save_canonical(
        &self,
        snapshot_id: &str,
        workflow_id: &str,
        state_json: String,
        created_at: Option<DateTime<Utc>>,
    ) -> Result<WorkflowSnapshot, PersistenceError> {
        let snapshot_id = validate_identifier(snapshot_id, "snapshot_id")?;
        let workflow_id = validate_identifier(workflow_id, "workflow_id")?;
        let digest = sha256_text(&state_json);
        let timestamp = truncate_to_micros(created_at.unwrap_or_else(Utc::now));
        let created_at_iso = timestamp_to_iso(&timestamp)?;
        self.database.transaction(|connection| {
            connection.execute(
                "
                INSERT INTO workflow_snapshots(
                    snapshot_id, workflow_id, state_json, digest, created_at
                ) VALUES (?1, ?2, ?3, ?4, ?5)
                ",
                params![
                    snapshot_id,
                    workflow_id,
                    state_json,
                    digest.to_hex(),
                    created_at_iso
                ],
            )?;
            let sequence = connection.last_insert_rowid();
            if sequence <= 0 {
                return Err(PersistenceError::SnapshotRowIdMissing);
            }
            Ok(WorkflowSnapshot {
                sequence,
                snapshot_id: snapshot_id.clone(),
                workflow_id: workflow_id.clone(),
                state_json: state_json.clone(),
                digest,
                created_at: timestamp,
            })
        })
    }

    /// Fetch one snapshot by id, verifying its bytes and digest.
    pub fn get(&self, snapshot_id: &str) -> Result<Option<WorkflowSnapshot>, PersistenceError> {
        let snapshot_id = validate_identifier(snapshot_id, "snapshot_id")?;
        self.database.serialized(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {SNAPSHOT_COLUMNS} FROM workflow_snapshots WHERE snapshot_id = ?1"
            ))?;
            let mut rows = statement.query(params![snapshot_id])?;
            match rows.next()? {
                Some(row) => snapshot_from_row(row).map(Some),
                None => Ok(None),
            }
        })
    }

    /// Fetch the newest snapshot for a workflow, verifying its bytes/digest.
    pub fn latest(&self, workflow_id: &str) -> Result<Option<WorkflowSnapshot>, PersistenceError> {
        let workflow_id = validate_identifier(workflow_id, "workflow_id")?;
        self.database.serialized(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {SNAPSHOT_COLUMNS} FROM workflow_snapshots \
                 WHERE workflow_id = ?1 ORDER BY sequence DESC LIMIT 1"
            ))?;
            let mut rows = statement.query(params![workflow_id])?;
            match rows.next()? {
                Some(row) => snapshot_from_row(row).map(Some),
                None => Ok(None),
            }
        })
    }

    /// Restore the canonical state JSON of one snapshot, or `None`.
    pub fn restore(&self, snapshot_id: &str) -> Result<Option<String>, PersistenceError> {
        Ok(self.get(snapshot_id)?.map(|snapshot| snapshot.state_json))
    }

    /// Restore the latest canonical state for a workflow, or `None`.
    pub fn restore_latest(&self, workflow_id: &str) -> Result<Option<String>, PersistenceError> {
        Ok(self
            .latest(workflow_id)?
            .map(|snapshot| snapshot.state_json))
    }
}

fn snapshot_from_row(row: &Row<'_>) -> Result<WorkflowSnapshot, PersistenceError> {
    let state_json: String = row.get("state_json")?;
    let canonical = canonicalize_json_object(&state_json)
        .map_err(|_| PersistenceError::StoredSnapshotNotCanonical)?;
    if canonical != state_json {
        return Err(PersistenceError::StoredSnapshotNotCanonical);
    }
    let stored_digest: String = row.get("digest")?;
    if !is_lowercase_sha256_hex(&stored_digest) {
        return Err(PersistenceError::SnapshotDigestVerification);
    }
    let digest = sha256_text(&canonical);
    if digest.to_hex() != stored_digest {
        return Err(PersistenceError::SnapshotDigestVerification);
    }
    let snapshot_id: String = row.get("snapshot_id")?;
    let workflow_id: String = row.get("workflow_id")?;
    let created_at: String = row.get("created_at")?;
    Ok(WorkflowSnapshot {
        sequence: row.get("sequence")?,
        snapshot_id: validate_identifier(&snapshot_id, "snapshot_id")?,
        workflow_id: validate_identifier(&workflow_id, "workflow_id")?,
        state_json,
        digest,
        created_at: timestamp_from_iso(&created_at)
            .map_err(|_| PersistenceError::StoredTimestamp)?,
    })
}
