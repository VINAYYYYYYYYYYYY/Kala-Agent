//! Native SQLite persistence compatible with `kala_agent.persistence`.
//!
//! This crate is the native Rust equivalent of the Python
//! `kala_agent.persistence.database`, `kala_agent.persistence.journal` and
//! `kala_agent.persistence.chain` modules:
//!
//! * [`Database`] owns one serialized SQLite writer, applies the frozen
//!   schema-v2 migrations, keeps `foreign_keys` on, keeps WAL journaling and
//!   SQLite's default durability, and nests transactions as savepoints.
//! * [`EventJournal`] appends genesis-rooted, SHA-256 chained events and fully
//!   verifies every stored row before every read.
//! * [`SnapshotStore`] stores canonical object JSON with its digest and
//!   verifies both on every read.
//!
//! Stored payloads use the larger
//! `kala_agent.contracts.models.CanonicalJsonObject` domain, never the
//! bounded `tools.protocol` tool limits; see [`persistence_limits`].
//!
//! Python is used only by the integration tests as an oracle. Production code
//! contains no Python, subprocess or fallback application.

#![forbid(unsafe_code)]

mod canonical;
mod database;
mod error;
mod journal;
mod schema;
mod snapshot;
mod time;

pub use canonical::{
    MAX_PERSISTENCE_JSON_DEPTH, canonicalize_json_object, canonicalize_json_object_bytes,
    canonicalize_object_value, is_lowercase_sha256_hex, parse_json_object, parse_json_object_bytes,
    parse_stored_digest, persistence_limits, sha256_text,
};
pub use database::{ConnectionGuard, Database, DatabaseConfig, MEMORY_PATH, TRANSACTION_SAVEPOINT};
pub use error::{JournalIntegrityError, PersistenceError};
pub use journal::{
    AppendOptions, CHAIN_COLUMNS, ChainVerification, Event, EventFilter, EventJournal,
};
pub use kala_json::{CanonicalValue, GENESIS_PREVIOUS_DIGEST, Limits, Sha256Digest};
pub use schema::{
    APPEND_ONLY_DELETE_TRIGGER, APPEND_ONLY_UPDATE_TRIGGER, CHAINED_INSERT_TRIGGER,
    MIGRATION_1_STEPS, MIGRATION_2_STEPS, MIGRATIONS, MigrationStep, SCHEMA_MIGRATIONS_TABLE,
    SchemaMigration, apply_migration, migrations,
};
pub use snapshot::{SnapshotStore, WorkflowSnapshot};
pub use time::{timestamp_from_iso, timestamp_to_iso, truncate_to_micros};
