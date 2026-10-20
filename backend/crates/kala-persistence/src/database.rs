//! The serialized SQLite writer.
//!
//! One connection is shared by every caller. SQLite allows one write
//! transaction per connection, so a single logical operation holds a reentrant
//! writer lock across all of its statements; a second thread cannot interleave
//! its own transaction with the first. Nested transactions become SAVEPOINTs
//! exactly like `kala_agent.persistence.database.Database.transaction`.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use chrono::Utc;
use kala_json::{CanonicalValue, Sha256Digest, canonical_dumps};
use parking_lot::{ReentrantMutex, ReentrantMutexGuard};
use rusqlite::{Connection, params};

use crate::canonical::{persistence_limits, sha256_text};
use crate::error::PersistenceError;
use crate::schema::{MIGRATIONS, SCHEMA_MIGRATIONS_TABLE, apply_migration};
use crate::time::{timestamp_to_iso, truncate_to_micros};

/// The in-memory database target SQLite and Python both special-case.
pub const MEMORY_PATH: &str = ":memory:";

/// The savepoint name used for nested transactions.
pub const TRANSACTION_SAVEPOINT: &str = "kala_agent_persistence";

/// Native open configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseConfig {
    /// `PRAGMA busy_timeout` in milliseconds.
    pub busy_timeout_ms: i64,
}

impl DatabaseConfig {
    /// Python `Database`'s default busy timeout.
    pub const DEFAULT_BUSY_TIMEOUT_MS: i64 = 5_000;

    /// Validate a busy timeout.
    pub fn new(busy_timeout_ms: i64) -> Result<Self, PersistenceError> {
        let config = Self { busy_timeout_ms };
        config.validate()?;
        Ok(config)
    }

    /// Check the signed 32-bit millisecond domain SQLite accepts.
    pub fn validate(&self) -> Result<(), PersistenceError> {
        if !(0..=2_147_483_647).contains(&self.busy_timeout_ms) {
            return Err(PersistenceError::InvalidBusyTimeout {
                value: self.busy_timeout_ms,
            });
        }
        Ok(())
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            busy_timeout_ms: Self::DEFAULT_BUSY_TIMEOUT_MS,
        }
    }
}

/// One SQLite database written by one serialized writer.
pub struct Database {
    path: PathBuf,
    busy_timeout_ms: i64,
    connection: ReentrantMutex<RefCell<Option<Connection>>>,
    active_guards: AtomicUsize,
}

impl Database {
    /// Open (or create) a database with the default busy timeout.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PersistenceError> {
        Self::open_with_config(path, DatabaseConfig::default())
    }

    /// Open an in-memory database.
    pub fn in_memory() -> Result<Self, PersistenceError> {
        Self::open(MEMORY_PATH)
    }

    /// Open a database with an explicit busy timeout in milliseconds.
    pub fn open_with_busy_timeout(
        path: impl AsRef<Path>,
        busy_timeout_ms: i64,
    ) -> Result<Self, PersistenceError> {
        Self::open_with_config(path, DatabaseConfig::new(busy_timeout_ms)?)
    }

    /// Open a database with a validated configuration.
    pub fn open_with_config(
        path: impl AsRef<Path>,
        config: DatabaseConfig,
    ) -> Result<Self, PersistenceError> {
        config.validate()?;
        let path = resolve_path(path.as_ref());
        let memory = path == Path::new(MEMORY_PATH);
        if !memory {
            prepare_parent_directory(&path)?;
        }
        let connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_millis(config.busy_timeout_ms as u64))?;
        connection.execute_batch("PRAGMA foreign_keys = ON")?;
        let _journal_mode: String =
            connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        if !memory {
            restrict_file_permissions(&path)?;
        }
        let database = Self {
            path,
            busy_timeout_ms: config.busy_timeout_ms,
            connection: ReentrantMutex::new(RefCell::new(Some(connection))),
            active_guards: AtomicUsize::new(0),
        };
        if let Err(error) = database.initialize() {
            let _ = database.close();
            return Err(error);
        }
        Ok(database)
    }

    /// The database target, including the literal `:memory:`.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The configured busy timeout in milliseconds.
    pub fn busy_timeout_ms(&self) -> i64 {
        self.busy_timeout_ms
    }

    /// Whether this database has been closed.
    pub fn closed(&self) -> bool {
        let guard = self.connection.lock();
        guard.borrow().is_none()
    }

    /// Apply every outstanding migration, atomically and in version order.
    pub fn initialize(&self) -> Result<(), PersistenceError> {
        let guard = self.connection.lock();
        let mut borrow = guard
            .try_borrow_mut()
            .map_err(|_| PersistenceError::ConnectionInUse)?;
        let connection = borrow.as_mut().ok_or(PersistenceError::Closed)?;
        connection.execute_batch(SCHEMA_MIGRATIONS_TABLE)?;
        connection.execute_batch("BEGIN IMMEDIATE")?;
        match apply_outstanding_migrations(connection) {
            Ok(()) => match connection.execute_batch("COMMIT") {
                Ok(()) => Ok(()),
                Err(error) => {
                    let _ = connection.execute_batch("ROLLBACK");
                    Err(error.into())
                }
            },
            Err(error) => {
                let _ = connection.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    /// The highest applied migration version, or `0` for an empty ledger.
    pub fn schema_version(&self) -> Result<i64, PersistenceError> {
        self.serialized(|connection| {
            let version: Option<i64> = connection.query_row(
                "SELECT MAX(version) AS version FROM schema_migrations",
                [],
                |row| row.get(0),
            )?;
            Ok(version.unwrap_or(0))
        })
    }

    /// SHA-256 of the whitespace-normalized `sqlite_master` schema.
    ///
    /// This is the corpus `schema_digest_definition`: all
    /// `[type, name, tbl_name, normalized sql]` entries ordered by
    /// `(type, name, tbl_name)`, serialized as canonical JSON.
    pub fn schema_digest(&self) -> Result<Sha256Digest, PersistenceError> {
        self.serialized(|connection| {
            let mut statement = connection.prepare(
                "SELECT type, name, tbl_name, sql FROM sqlite_master \
                 ORDER BY type, name, tbl_name",
            )?;
            let mut rows = statement.query([])?;
            let mut entries: Vec<CanonicalValue> = Vec::new();
            while let Some(row) = rows.next()? {
                let entry_type: String = row.get(0)?;
                let name: String = row.get(1)?;
                let table_name: String = row.get(2)?;
                let sql: Option<String> = row.get(3)?;
                let normalized = sql
                    .as_deref()
                    .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "));
                entries.push(CanonicalValue::array([
                    CanonicalValue::from(entry_type),
                    CanonicalValue::from(name),
                    CanonicalValue::from(table_name),
                    match normalized {
                        Some(text) => CanonicalValue::from(text),
                        None => CanonicalValue::Null,
                    },
                ]));
            }
            let rendered = canonical_dumps(&CanonicalValue::Array(entries), &persistence_limits())?;
            Ok(sha256_text(&rendered))
        })
    }

    /// Hold the writer lock for the duration of one fallible closure.
    pub fn serialized<T>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, PersistenceError>,
    ) -> Result<T, PersistenceError> {
        let guard = self.connection.lock();
        let borrow = guard.borrow();
        let connection = borrow.as_ref().ok_or(PersistenceError::Closed)?;
        operation(connection)
    }

    /// Hold the writer lock for the duration of one infallible closure.
    pub fn with_connection<T>(
        &self,
        operation: impl FnOnce(&Connection) -> T,
    ) -> Result<T, PersistenceError> {
        self.serialized(|connection| Ok(operation(connection)))
    }

    /// Execute a semicolon-delimited SQL script on the serialized writer.
    pub fn execute_batch(&self, sql: &str) -> Result<(), PersistenceError> {
        self.serialized(|connection| {
            connection.execute_batch(sql)?;
            Ok(())
        })
    }

    /// Run a write transaction, or a savepoint when one is already open.
    ///
    /// The closure receives the shared connection; every nested call through
    /// the same `Database` re-enters the writer lock and nests a savepoint.
    pub fn transaction<T>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T, PersistenceError>,
    ) -> Result<T, PersistenceError> {
        let guard = self.connection.lock();
        let borrow = guard.borrow();
        let connection = borrow.as_ref().ok_or(PersistenceError::Closed)?;
        let nested = !connection.is_autocommit();
        if nested {
            connection.execute_batch(&format!("SAVEPOINT {TRANSACTION_SAVEPOINT}"))?;
        } else {
            connection.execute_batch("BEGIN IMMEDIATE")?;
        }
        // The guard rolls back on every exit that did not reach a successful
        // COMMIT/RELEASE: operation errors, COMMIT/RELEASE errors and Rust
        // panics unwinding through the closure. A failed COMMIT often leaves
        // the transaction live; without this the next caller would silently
        // become a savepoint inside the abandoned transaction.
        let mut state = TransactionState {
            connection,
            nested,
            finished: false,
        };
        match operation(connection) {
            Ok(value) => {
                state.finish()?;
                Ok(value)
            }
            Err(error) => {
                state.rollback();
                Err(error)
            }
        }
    }

    /// Close the connection. Idempotent; later access fails with `Closed`.
    ///
    /// A live [`ConnectionGuard`] prevents closing explicitly instead of
    /// leaving the guard with a vanished connection. This also makes a
    /// same-thread reentrant `close` while a guard is held fail with
    /// [`PersistenceError::ConnectionInUse`] rather than panicking later.
    pub fn close(&self) -> Result<(), PersistenceError> {
        let guard = self.connection.lock();
        if self.active_guards.load(Ordering::SeqCst) > 0 {
            return Err(PersistenceError::ConnectionInUse);
        }
        let mut borrow = guard
            .try_borrow_mut()
            .map_err(|_| PersistenceError::ConnectionInUse)?;
        let Some(connection) = borrow.take() else {
            return Ok(());
        };
        if !connection.is_autocommit() {
            let _ = connection.execute_batch("ROLLBACK");
        }
        connection
            .close()
            .map_err(|(_connection, error)| error.into())
    }

    /// Read a guard to the open connection, re-entering the writer lock.
    pub fn connection(&self) -> Result<ConnectionGuard<'_>, PersistenceError> {
        let guard = self.connection.lock();
        if guard.borrow().is_none() {
            return Err(PersistenceError::Closed);
        }
        self.active_guards.fetch_add(1, Ordering::SeqCst);
        Ok(ConnectionGuard {
            guard,
            active_guards: &self.active_guards,
        })
    }
}

/// One started `BEGIN IMMEDIATE` transaction or nested `SAVEPOINT`.
///
/// Every path that does not reach a successful `COMMIT`/`RELEASE` calls
/// [`TransactionState::rollback`], including unwind. Rollback errors are
/// deliberately ignored: the operation's original error, or the failing
/// COMMIT/RELEASE error, is the error the caller must see.
struct TransactionState<'a> {
    connection: &'a Connection,
    nested: bool,
    finished: bool,
}

impl TransactionState<'_> {
    fn finish(&mut self) -> Result<(), PersistenceError> {
        let result = if self.nested {
            self.connection
                .execute_batch(&format!("RELEASE SAVEPOINT {TRANSACTION_SAVEPOINT}"))
        } else {
            self.connection.execute_batch("COMMIT")
        };
        match result {
            Ok(()) => {
                self.finished = true;
                Ok(())
            }
            Err(error) => {
                // A failed COMMIT may still leave the transaction open; a
                // failed RELEASE may leave the savepoint open. Clean up before
                // reporting the commit/release error.
                self.rollback();
                Err(error.into())
            }
        }
    }

    fn rollback(&mut self) {
        if self.finished {
            return;
        }
        if self.nested {
            let _ = self
                .connection
                .execute_batch(&format!("ROLLBACK TO SAVEPOINT {TRANSACTION_SAVEPOINT}"));
            let _ = self
                .connection
                .execute_batch(&format!("RELEASE SAVEPOINT {TRANSACTION_SAVEPOINT}"));
        } else {
            let _ = self.connection.execute_batch("ROLLBACK");
        }
        self.finished = true;
    }
}

impl Drop for TransactionState<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.rollback();
        }
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

impl std::fmt::Debug for Database {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Database")
            .field("path", &self.path)
            .field("busy_timeout_ms", &self.busy_timeout_ms)
            .field("closed", &self.closed())
            .finish()
    }
}

/// A reentrant, immutable borrow of the open connection.
///
/// While a guard exists, [`Database::close`] fails with
/// [`PersistenceError::ConnectionInUse`] instead of removing the connection
/// from under the guard.
pub struct ConnectionGuard<'a> {
    guard: ReentrantMutexGuard<'a, RefCell<Option<Connection>>>,
    active_guards: &'a AtomicUsize,
}

impl ConnectionGuard<'_> {
    /// Run a closure with the borrowed connection.
    pub fn with<T>(&self, operation: impl FnOnce(&Connection) -> T) -> T {
        let borrow = self.guard.borrow();
        let connection = borrow.as_ref().expect("connection checked by Database");
        operation(connection)
    }
}

impl Drop for ConnectionGuard<'_> {
    fn drop(&mut self) {
        self.active_guards.fetch_sub(1, Ordering::SeqCst);
    }
}

fn apply_outstanding_migrations(connection: &Connection) -> Result<(), PersistenceError> {
    let mut statement =
        connection.prepare("SELECT version FROM schema_migrations ORDER BY version")?;
    let mut rows = statement.query([])?;
    let mut applied_versions: Vec<i64> = Vec::new();
    while let Some(row) = rows.next()? {
        applied_versions.push(row.get(0)?);
    }
    drop(rows);
    drop(statement);
    if applied_versions
        .iter()
        .any(|version| !MIGRATIONS.iter().any(|item| item.version == *version))
    {
        return Err(PersistenceError::SchemaNewer);
    }
    for migration in MIGRATIONS {
        if applied_versions.contains(&migration.version) {
            continue;
        }
        apply_migration(connection, migration)?;
        let applied_at = timestamp_to_iso(&truncate_to_micros(Utc::now()))?;
        connection.execute(
            "INSERT INTO schema_migrations(version, name, applied_at) VALUES (?, ?, ?)",
            params![migration.version, migration.name, applied_at],
        )?;
    }
    Ok(())
}

fn resolve_path(path: &Path) -> PathBuf {
    let Some(text) = path.to_str() else {
        return path.to_path_buf();
    };
    if text == "~" {
        if let Some(home) = home_directory() {
            return home;
        }
    }
    if let Some(rest) = text.strip_prefix("~/") {
        if let Some(home) = home_directory() {
            return home.join(rest);
        }
    }
    #[cfg(windows)]
    if let Some(rest) = text.strip_prefix("~\\") {
        if let Some(home) = home_directory() {
            return home.join(rest);
        }
    }
    path.to_path_buf()
}

fn home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
    #[cfg(not(windows))]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

fn prepare_parent_directory(path: &Path) -> Result<(), PersistenceError> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    let existed = parent.exists();
    fs::create_dir_all(parent)?;
    #[cfg(unix)]
    if !existed {
        use std::os::unix::fs::PermissionsExt;
        // Only a directory this process created is tightened here. Existing
        // ancestor directories are never chmod-ed as a shortcut.
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn restrict_file_permissions(path: &Path) -> Result<(), PersistenceError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Database>();
    }

    #[test]
    fn memory_database_initializes() {
        let database = Database::in_memory().expect("open");
        assert_eq!(database.schema_version().expect("version"), 2);
        assert!(!database.closed());
        database.close().expect("close");
        assert!(database.closed());
    }
}
