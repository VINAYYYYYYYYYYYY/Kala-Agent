//! The ordered, versioned SQLite schema migrations.
//!
//! These are the native equivalents of `kala_agent.persistence.database`'s
//! `SchemaMigration` values and `_STATE_MIGRATION` tuple. The SQL text, table
//! and index names, trigger names and migration version ordering are preserved
//! so the whitespace-normalized `sqlite_master` schema digest of a freshly
//! initialized database is byte-identical to the Python-created corpus
//! database (`backend/fixtures/corpus/v1/schema_v2.sqlite3`).

use rusqlite::Connection;

use crate::error::PersistenceError;

/// `CREATE TABLE` for the migration ledger, exactly as `Database.initialize`
/// writes it.
pub const SCHEMA_MIGRATIONS_TABLE: &str = "
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    name TEXT NOT NULL,
                    applied_at TEXT NOT NULL
                )
                ";

/// The append-only update trigger created by migration 1.
pub const APPEND_ONLY_UPDATE_TRIGGER: &str = "
    CREATE TRIGGER events_prevent_update
    BEFORE UPDATE ON events
    BEGIN
        SELECT RAISE(ABORT, 'events are append-only');
    END
    ";

/// The append-only delete trigger created by migration 1.
pub const APPEND_ONLY_DELETE_TRIGGER: &str = "
    CREATE TRIGGER events_prevent_delete
    BEFORE DELETE ON events
    BEGIN
        SELECT RAISE(ABORT, 'events are append-only');
    END
    ";

/// The chained-insert guard created by migration 2.
pub const CHAINED_INSERT_TRIGGER: &str = "
    CREATE TRIGGER events_require_event_chain
    BEFORE INSERT ON events
    WHEN length(NEW.previous_event_digest) <> 64
      OR length(NEW.event_digest) <> 64
      OR NEW.previous_event_digest <> lower(NEW.previous_event_digest)
      OR NEW.event_digest <> lower(NEW.event_digest)
    BEGIN
        SELECT RAISE(ABORT, 'events require a hash chain');
    END
    ";

/// One step of a migration: a statement, or the chain backfill callable.
#[derive(Debug, Clone, Copy)]
pub enum MigrationStep {
    /// A SQL statement applied to the open connection.
    Sql(&'static str),
    /// `kala_agent.persistence.chain.backfill_event_chain`.
    BackfillEventChain,
}

/// One ordered, versioned schema change.
#[derive(Debug, Clone, Copy)]
pub struct SchemaMigration {
    /// One-based migration version.
    pub version: i64,
    /// Migration name recorded in `schema_migrations`.
    pub name: &'static str,
    /// Ordered steps.
    pub steps: &'static [MigrationStep],
}

/// Migration 1, `initial_persistence_schema`, exactly as in the Python source.
pub static MIGRATION_1_STEPS: &[MigrationStep] = &[
    MigrationStep::Sql(
        "
            CREATE TABLE events (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                event_type TEXT NOT NULL,
                aggregate_id TEXT NOT NULL,
                workflow_id TEXT,
                run_id TEXT,
                payload_json TEXT NOT NULL,
                digest TEXT NOT NULL,
                occurred_at TEXT NOT NULL
            )
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE INDEX events_type_sequence_idx
            ON events(event_type, sequence)
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE INDEX events_aggregate_sequence_idx
            ON events(aggregate_id, sequence)
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE INDEX events_workflow_sequence_idx
            ON events(workflow_id, sequence)
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE INDEX events_run_sequence_idx
            ON events(run_id, sequence)
            ",
    ),
    MigrationStep::Sql(APPEND_ONLY_UPDATE_TRIGGER),
    MigrationStep::Sql(APPEND_ONLY_DELETE_TRIGGER),
    MigrationStep::Sql(
        "
            CREATE TABLE design_specs (
                id TEXT PRIMARY KEY,
                model_json TEXT NOT NULL
            )
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE TABLE design_revisions (
                revision_id TEXT PRIMARY KEY,
                design_id TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                model_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                UNIQUE(design_id, sequence)
            )
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE INDEX design_revisions_design_sequence_idx
            ON design_revisions(design_id, sequence)
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE TABLE approval_requests (
                request_id TEXT PRIMARY KEY,
                workflow_id TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                model_json TEXT NOT NULL
            )
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE INDEX approval_requests_workflow_created_idx
            ON approval_requests(workflow_id, created_at, request_id)
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE TABLE tool_calls (
                call_id TEXT PRIMARY KEY,
                tool_id TEXT NOT NULL,
                issued_at TEXT NOT NULL,
                model_json TEXT NOT NULL
            )
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE INDEX tool_calls_tool_issued_idx
            ON tool_calls(tool_id, issued_at, call_id)
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE TABLE tool_results (
                call_id TEXT PRIMARY KEY,
                tool_id TEXT NOT NULL,
                status TEXT NOT NULL,
                model_json TEXT NOT NULL,
                FOREIGN KEY(call_id) REFERENCES tool_calls(call_id)
                    ON UPDATE RESTRICT ON DELETE RESTRICT
            )
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE INDEX tool_results_tool_status_idx
            ON tool_results(tool_id, status, call_id)
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE TABLE memory_entries (
                memory_id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                provenance_id TEXT,
                created_at TEXT NOT NULL,
                expires_at TEXT,
                model_json TEXT NOT NULL
            )
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE INDEX memory_entries_source_created_idx
            ON memory_entries(source, created_at, memory_id)
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE INDEX memory_entries_expiry_idx
            ON memory_entries(expires_at)
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE TABLE workflow_snapshots (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                snapshot_id TEXT NOT NULL UNIQUE,
                workflow_id TEXT NOT NULL,
                state_json TEXT NOT NULL,
                digest TEXT NOT NULL,
                created_at TEXT NOT NULL
            )
            ",
    ),
    MigrationStep::Sql(
        "
            CREATE INDEX workflow_snapshots_workflow_sequence_idx
            ON workflow_snapshots(workflow_id, sequence)
            ",
    ),
];

/// Migration 2, `journal_event_hash_chain`, exactly as in the Python source.
pub static MIGRATION_2_STEPS: &[MigrationStep] = &[
    MigrationStep::Sql(
        "ALTER TABLE events ADD COLUMN previous_event_digest TEXT NOT NULL DEFAULT ''",
    ),
    MigrationStep::Sql("ALTER TABLE events ADD COLUMN event_digest TEXT NOT NULL DEFAULT ''"),
    MigrationStep::Sql("DROP TRIGGER IF EXISTS events_prevent_update"),
    MigrationStep::BackfillEventChain,
    MigrationStep::Sql(APPEND_ONLY_UPDATE_TRIGGER),
    MigrationStep::Sql(CHAINED_INSERT_TRIGGER),
];

/// Every migration, in version order.
pub static MIGRATIONS: &[SchemaMigration] = &[
    SchemaMigration {
        version: 1,
        name: "initial_persistence_schema",
        steps: MIGRATION_1_STEPS,
    },
    SchemaMigration {
        version: 2,
        name: "journal_event_hash_chain",
        steps: MIGRATION_2_STEPS,
    },
];

/// The migration table in version order.
pub fn migrations() -> &'static [SchemaMigration] {
    MIGRATIONS
}

/// Apply one migration's steps to the open connection.
///
/// The caller owns the surrounding transaction, exactly like
/// `Database._apply_migration`.
pub fn apply_migration(
    connection: &Connection,
    migration: &SchemaMigration,
) -> Result<(), PersistenceError> {
    for step in migration.steps {
        match step {
            MigrationStep::Sql(statement) => {
                connection.execute_batch(statement)?;
            }
            MigrationStep::BackfillEventChain => {
                crate::journal::backfill_event_chain(connection)?;
            }
        }
    }
    Ok(())
}
