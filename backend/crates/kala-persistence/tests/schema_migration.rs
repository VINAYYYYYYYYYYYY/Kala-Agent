//! Schema v2 initialization, migration 1 to 2 backfill and database behavior.

mod support;

use std::path::Path;

use kala_persistence::{
    Database, DatabaseConfig, JournalIntegrityError, MIGRATIONS, PersistenceError,
};
use rusqlite::Connection;
use support::{copy_fixture_database, fixed_time, fixture_json};

fn build_schema1(path: &Path, mode: Schema1Mode) {
    let connection = Connection::open(path).expect("open schema1");
    connection
        .execute_batch(kala_persistence::SCHEMA_MIGRATIONS_TABLE)
        .expect("create migration ledger");
    for step in kala_persistence::MIGRATION_1_STEPS {
        if let kala_persistence::MigrationStep::Sql(statement) = step {
            connection
                .execute_batch(statement)
                .expect("apply migration 1 step");
        } else {
            panic!("migration 1 has no callable steps");
        }
    }
    connection
        .execute(
            "INSERT INTO schema_migrations(version, name, applied_at) VALUES (1, ?, ?)",
            rusqlite::params!["initial_persistence_schema", "2026-01-01T00:00:00.000000Z"],
        )
        .expect("record migration 1");
    let payloads = [r#"{"a":1}"#, r#"{"a":2}"#, r#"{"a":3}"#];
    for (index, payload) in payloads.iter().enumerate() {
        let sequence = index as i64 + 1;
        if mode == Schema1Mode::Gap && sequence == 2 {
            continue;
        }
        connection
            .execute(
                "INSERT INTO events(
                    sequence, event_type, aggregate_id, workflow_id, run_id, payload_json,
                    digest, occurred_at
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    sequence,
                    format!("step.{index}"),
                    format!("step-{index}"),
                    "workflow-1",
                    "run-1",
                    payload,
                    kala_persistence::sha256_text(payload).to_hex(),
                    fixed_time(index as i64).to_rfc3339(),
                ],
            )
            .expect("insert schema1 event");
    }
    if mode == Schema1Mode::CorruptPayload {
        connection
            .execute_batch("DROP TRIGGER events_prevent_update")
            .expect("drop update trigger");
        connection
            .execute(
                "UPDATE events SET payload_json = ?1 WHERE sequence = 2",
                rusqlite::params![r#"{"a":99}"#],
            )
            .expect("corrupt payload");
    }
    drop(connection);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Schema1Mode {
    Healthy,
    CorruptPayload,
    Gap,
}

#[test]
fn fresh_database_matches_fixture_schema_digest() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("fresh.sqlite3");
    let database = Database::open(&path).expect("open fresh database");
    assert_eq!(database.schema_version().expect("version"), 2);
    assert_eq!(
        database.busy_timeout_ms(),
        DatabaseConfig::DEFAULT_BUSY_TIMEOUT_MS
    );
    database
        .serialized(|connection| {
            let foreign_keys: i64 =
                connection.query_row("PRAGMA foreign_keys", [], |row| row.get(0))?;
            assert_eq!(foreign_keys, 1);
            let journal_mode: String =
                connection.query_row("PRAGMA journal_mode", [], |row| row.get(0))?;
            assert_eq!(journal_mode, "wal");
            let busy_timeout: i64 =
                connection.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
            assert_eq!(busy_timeout, DatabaseConfig::DEFAULT_BUSY_TIMEOUT_MS);
            let migrations: Vec<(i64, String)> = connection
                .prepare("SELECT version, name FROM schema_migrations ORDER BY version")?
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<Result<_, _>>()?;
            assert_eq!(
                migrations,
                vec![
                    (1, "initial_persistence_schema".to_owned()),
                    (2, "journal_event_hash_chain".to_owned()),
                ]
            );
            let tables: Vec<String> = connection
                .prepare(
                    "SELECT name FROM sqlite_master WHERE type = 'table' \
                     AND name NOT LIKE 'sqlite_%' ORDER BY name",
                )?
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            assert_eq!(
                tables,
                vec![
                    "approval_requests",
                    "design_revisions",
                    "design_specs",
                    "events",
                    "memory_entries",
                    "schema_migrations",
                    "tool_calls",
                    "tool_results",
                    "workflow_snapshots",
                ]
            );
            let triggers: Vec<String> = connection
                .prepare("SELECT name FROM sqlite_master WHERE type = 'trigger' ORDER BY name")?
                .query_map([], |row| row.get(0))?
                .collect::<Result<_, _>>()?;
            assert_eq!(
                triggers,
                vec![
                    "events_prevent_delete",
                    "events_prevent_update",
                    "events_require_event_chain",
                ]
            );
            Ok(())
        })
        .expect("schema checks");

    let digest = database.schema_digest().expect("schema digest");
    let fixture = fixture_json("event_chain.json");
    let expected = fixture["database"]["schema_digest"]
        .as_str()
        .expect("fixture schema digest");
    assert_eq!(digest.to_hex(), expected, "fresh schema digest drifted");

    // The Python-created fixture must produce the identical digest when read
    // through the native schema digest implementation.
    let copy = copy_fixture_database();
    let fixture_database = Database::open(&copy.path).expect("open fixture copy");
    assert_eq!(
        fixture_database
            .schema_digest()
            .expect("fixture digest")
            .to_hex(),
        expected
    );
    assert_eq!(fixture_database.schema_version().expect("version"), 2);
}

#[test]
fn migration_chains_pre_existing_journal_and_appends() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("pre-chain.sqlite3");
    build_schema1(&path, Schema1Mode::Healthy);

    let database = Database::open(&path).expect("migrate healthy");
    assert_eq!(database.schema_version().expect("version"), 2);
    let journal = kala_persistence::EventJournal::new(&database).expect("journal");
    let events = journal.read().expect("read");
    assert_eq!(
        events
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(
        events
            .iter()
            .map(|event| event.payload_json.clone())
            .collect::<Vec<_>>(),
        vec![r#"{"a":1}"#, r#"{"a":2}"#, r#"{"a":3}"#]
    );
    assert_eq!(
        events[0].previous_event_digest,
        kala_persistence::GENESIS_PREVIOUS_DIGEST
    );
    assert_eq!(events[1].previous_event_digest, events[0].event_digest);
    assert_eq!(events[2].previous_event_digest, events[1].event_digest);
    let appended = journal
        .append("step.migrated", "step-3", r#"{"a":4}"#)
        .expect("append after migration");
    assert_eq!(appended.sequence, 4);
    assert_eq!(appended.previous_event_digest, events[2].event_digest);
    assert_eq!(
        journal.verify_chain().expect("verify").head_event_digest,
        Some(appended.event_digest)
    );
}

#[test]
fn migration_refuses_corrupt_pre_chain_history_and_rolls_back() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("pre-chain-corrupt.sqlite3");
    build_schema1(&path, Schema1Mode::CorruptPayload);

    match Database::open(&path) {
        Err(PersistenceError::JournalIntegrity(JournalIntegrityError::BackfillPayload {
            sequence: 2,
        })) => {}
        other => panic!("expected backfill payload refusal, got {other:?}"),
    }
    let connection = Connection::open(&path).expect("reopen raw");
    let version: Option<i64> = connection
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("version");
    assert_eq!(version, Some(1));
    let columns: Vec<String> = connection
        .prepare("PRAGMA table_info(events)")
        .expect("prepare table_info")
        .query_map([], |row| row.get::<_, String>(1))
        .expect("query columns")
        .collect::<Result<_, _>>()
        .expect("collect columns");
    assert_eq!(
        columns,
        vec![
            "sequence",
            "event_type",
            "aggregate_id",
            "workflow_id",
            "run_id",
            "payload_json",
            "digest",
            "occurred_at",
        ]
    );
    let payload: String = connection
        .query_row(
            "SELECT payload_json FROM events WHERE sequence = 2",
            [],
            |row| row.get(0),
        )
        .expect("payload");
    assert_eq!(payload, r#"{"a":99}"#);
    // The rolled-back migration must not have left the migration-2 trigger
    // behind, and the pre-existing delete trigger must still be intact.
    let insert_trigger: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' \
             AND name = 'events_require_event_chain'",
            [],
            |row| row.get(0),
        )
        .expect("insert trigger count");
    assert_eq!(insert_trigger, 0);
    let delete_trigger: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'trigger' \
             AND name = 'events_prevent_delete'",
            [],
            |row| row.get(0),
        )
        .expect("delete trigger count");
    assert_eq!(delete_trigger, 1);
}

#[test]
fn migration_refuses_a_sequence_gap() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("pre-chain-gap.sqlite3");
    build_schema1(&path, Schema1Mode::Gap);
    match Database::open(&path) {
        Err(PersistenceError::JournalIntegrity(JournalIntegrityError::BackfillGap {
            expected: 2,
        })) => {}
        other => panic!("expected backfill gap refusal, got {other:?}"),
    }
}

#[test]
fn newer_schema_and_invalid_configuration_are_refused() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("newer.sqlite3");
    {
        let database = Database::open(&path).expect("open");
        database
            .execute_batch(
                "INSERT INTO schema_migrations(version, name, applied_at)
                 VALUES (99, 'future', '2026-01-01T00:00:00.000000Z')",
            )
            .expect("insert future migration");
    }
    match Database::open(&path) {
        Err(PersistenceError::SchemaNewer) => {}
        other => panic!("expected SchemaNewer, got {other:?}"),
    }

    assert!(matches!(
        DatabaseConfig::new(-1),
        Err(PersistenceError::InvalidBusyTimeout { .. })
    ));
    assert!(matches!(
        DatabaseConfig::new(2_147_483_648),
        Err(PersistenceError::InvalidBusyTimeout { .. })
    ));
    assert!(DatabaseConfig::new(0).is_ok());
    assert!(DatabaseConfig::new(2_147_483_647).is_ok());
}

#[test]
fn busy_timeout_close_and_closed_access() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("timeout.sqlite3");
    let database = Database::open_with_busy_timeout(&path, 7_500).expect("open");
    assert_eq!(database.busy_timeout_ms(), 7_500);
    assert_eq!(
        database
            .serialized(|connection| {
                let value: i64 =
                    connection.query_row("PRAGMA busy_timeout", [], |row| row.get(0))?;
                Ok(value)
            })
            .expect("busy timeout"),
        7_500
    );
    database.close().expect("close");
    database.close().expect("idempotent close");
    assert!(database.closed());
    assert!(matches!(
        database.schema_version(),
        Err(PersistenceError::Closed)
    ));
    assert!(matches!(
        database.serialized(|_| Ok(())),
        Err(PersistenceError::Closed)
    ));
}

#[test]
fn migrations_are_ordered_and_named() {
    assert_eq!(MIGRATIONS.len(), 2);
    assert_eq!(MIGRATIONS[0].version, 1);
    assert_eq!(MIGRATIONS[0].name, "initial_persistence_schema");
    assert_eq!(MIGRATIONS[1].version, 2);
    assert_eq!(MIGRATIONS[1].name, "journal_event_hash_chain");
}

#[cfg(unix)]
#[test]
fn private_directory_and_file_modes() {
    use std::os::unix::fs::PermissionsExt;

    let directory = tempfile::TempDir::new().expect("temp dir");
    let parent = directory.path().join("private");
    let path = parent.join("kala.sqlite3");
    let database = Database::open(&path).expect("open");
    assert_eq!(
        std::fs::metadata(&parent)
            .expect("parent")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(&path).expect("file").permissions().mode() & 0o777,
        0o600
    );
    database.close().expect("close");
}

#[test]
fn transaction_rolls_back_and_nests_savepoints() {
    let database = Database::in_memory().expect("open");
    database
        .transaction(|connection| {
            connection.execute(
                "INSERT INTO design_specs(id, model_json) VALUES ('spec-1', '{}')",
                [],
            )?;
            Ok(())
        })
        .expect("commit");
    assert!(
        database
            .transaction(|_| Err::<(), _>(PersistenceError::InvalidLimit))
            .is_err()
    );
    assert_eq!(count_design_specs(&database), 1);

    database
        .transaction(|connection| {
            connection.execute(
                "INSERT INTO design_specs(id, model_json) VALUES ('spec-2', '{}')",
                [],
            )?;
            let nested = database.transaction(|connection| {
                connection.execute(
                    "INSERT INTO design_specs(id, model_json) VALUES ('spec-3', '{}')",
                    [],
                )?;
                Err::<(), _>(PersistenceError::InvalidLimit)
            });
            assert!(nested.is_err());
            Ok(())
        })
        .expect("outer commit");
    assert_eq!(count_design_specs(&database), 2);
    assert_eq!(
        database
            .serialized(|connection| {
                let count: i64 = connection.query_row(
                    "SELECT COUNT(*) FROM design_specs WHERE id = 'spec-3'",
                    [],
                    |row| row.get(0),
                )?;
                Ok(count)
            })
            .expect("count"),
        0
    );
}

#[test]
fn failed_commit_rolls_back_and_leaves_autocommit() {
    let database = Database::in_memory().expect("open");
    database
        .execute_batch(
            "CREATE TABLE review_parent(id INTEGER PRIMARY KEY);
             CREATE TABLE review_child(
                 parent_id INTEGER REFERENCES review_parent(id)
                     DEFERRABLE INITIALLY DEFERRED
             );",
        )
        .expect("create deferred schema");

    let failed = database.transaction(|connection| {
        connection.execute("INSERT INTO review_child(parent_id) VALUES (123)", [])?;
        Ok(())
    });
    assert!(
        matches!(failed, Err(PersistenceError::Sqlite(_))),
        "expected deferred-FK COMMIT failure, got {failed:?}"
    );
    assert!(
        database
            .serialized(|connection| Ok(connection.is_autocommit()))
            .expect("autocommit state"),
        "failed COMMIT must not leave a live outer transaction"
    );
    assert_eq!(count_table(&database, "review_child"), 0);

    // The next transaction must be a real BEGIN IMMEDIATE/COMMIT, not a
    // savepoint inside the abandoned transaction.
    database
        .transaction(|connection| {
            connection.execute(
                "INSERT INTO design_specs(id, model_json) VALUES ('spec-after-fail', '{}')",
                [],
            )?;
            Ok(())
        })
        .expect("transaction after failed commit");
    assert_eq!(count_design_specs(&database), 1);
    assert!(
        database
            .serialized(|connection| Ok(connection.is_autocommit()))
            .expect("autocommit after success")
    );
}

#[test]
fn failed_nested_release_preserves_the_outer_transaction() {
    let database = Database::in_memory().expect("open");
    database
        .transaction(|connection| {
            connection.execute(
                "INSERT INTO design_specs(id, model_json) VALUES ('spec-outer', '{}')",
                [],
            )?;
            let nested = database.transaction(|nested| {
                // An operation that destroys its own savepoint makes the
                // transaction finalizer's RELEASE fail.
                nested.execute_batch("RELEASE SAVEPOINT kala_agent_persistence")?;
                Ok(())
            });
            assert!(
                matches!(nested, Err(PersistenceError::Sqlite(_))),
                "expected RELEASE failure, got {nested:?}"
            );
            // Cleanup of the failed savepoint must not roll back the outer
            // transaction or leave it in a broken state.
            assert!(
                !connection.is_autocommit(),
                "outer transaction must remain live"
            );
            connection.execute(
                "INSERT INTO design_specs(id, model_json) VALUES ('spec-outer-2', '{}')",
                [],
            )?;
            Ok(())
        })
        .expect("outer commit after failed nested RELEASE");
    assert_eq!(count_design_specs(&database), 2);
}

#[test]
fn outer_rollback_undoes_released_savepoint_work() {
    let database = Database::in_memory().expect("open");
    let result = database.transaction(|connection| {
        connection.execute(
            "INSERT INTO design_specs(id, model_json) VALUES ('spec-outer', '{}')",
            [],
        )?;
        database
            .transaction(|nested| {
                nested.execute(
                    "INSERT INTO design_specs(id, model_json) VALUES ('spec-inner', '{}')",
                    [],
                )?;
                Ok(())
            })
            .expect("nested release");
        Err::<(), _>(PersistenceError::InvalidLimit)
    });
    assert!(matches!(result, Err(PersistenceError::InvalidLimit)));
    assert_eq!(count_design_specs(&database), 0);
    assert!(
        database
            .serialized(|connection| Ok(connection.is_autocommit()))
            .expect("autocommit state")
    );
}

#[test]
fn panic_unwind_rolls_back_and_the_database_is_reusable() {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    let database = Database::in_memory().expect("open");
    let panicked = catch_unwind(AssertUnwindSafe(|| {
        let _ = database.transaction(|connection| -> Result<(), PersistenceError> {
            connection.execute(
                "INSERT INTO design_specs(id, model_json) VALUES ('spec-panic', '{}')",
                [],
            )?;
            panic!("review panic");
        });
    }));
    assert!(panicked.is_err(), "the closure was expected to panic");
    assert!(
        database
            .serialized(|connection| Ok(connection.is_autocommit()))
            .expect("autocommit state"),
        "panic unwind must roll back the live transaction"
    );
    assert_eq!(count_design_specs(&database), 0);

    database
        .transaction(|connection| {
            connection.execute(
                "INSERT INTO design_specs(id, model_json) VALUES ('spec-after-panic', '{}')",
                [],
            )?;
            Ok(())
        })
        .expect("transaction after panic");
    assert_eq!(count_design_specs(&database), 1);
}

#[test]
fn panic_in_nested_savepoint_rolls_back_only_the_nested_work() {
    use std::panic::{AssertUnwindSafe, catch_unwind};

    let database = Database::in_memory().expect("open");
    database
        .transaction(|connection| {
            connection.execute(
                "INSERT INTO design_specs(id, model_json) VALUES ('spec-live', '{}')",
                [],
            )?;
            let nested = catch_unwind(AssertUnwindSafe(|| {
                let _ = database.transaction(|nested| -> Result<(), PersistenceError> {
                    nested.execute(
                        "INSERT INTO design_specs(id, model_json) VALUES ('spec-dead', '{}')",
                        [],
                    )?;
                    panic!("nested review panic");
                });
            }));
            assert!(nested.is_err(), "nested closure was expected to panic");
            assert!(
                !connection.is_autocommit(),
                "outer transaction must survive a nested panic"
            );
            connection.execute(
                "INSERT INTO design_specs(id, model_json) VALUES ('spec-live-2', '{}')",
                [],
            )?;
            Ok(())
        })
        .expect("outer commit after nested panic");
    assert_eq!(count_design_specs(&database), 2);
    assert_eq!(
        database
            .serialized(|connection| {
                let count: i64 = connection.query_row(
                    "SELECT COUNT(*) FROM design_specs WHERE id = 'spec-dead'",
                    [],
                    |row| row.get(0),
                )?;
                Ok(count)
            })
            .expect("count"),
        0
    );
}

#[test]
fn close_is_refused_while_a_connection_guard_is_held() {
    let database = Database::in_memory().expect("open");
    let guard = database.connection().expect("connection guard");
    assert!(matches!(
        database.close(),
        Err(PersistenceError::ConnectionInUse)
    ));
    assert_eq!(
        guard
            .with(|connection| connection
                .query_row("PRAGMA foreign_keys", [], |row| row.get::<_, i64>(0)))
            .expect("query through held guard"),
        1
    );
    drop(guard);
    database.close().expect("close after guard drop");
    assert!(database.closed());
    assert!(matches!(
        database.connection(),
        Err(PersistenceError::Closed)
    ));
}

fn count_table(database: &Database, table: &str) -> i64 {
    database
        .serialized(|connection| {
            let count: i64 =
                connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })?;
            Ok(count)
        })
        .expect("count")
}

fn count_design_specs(database: &Database) -> i64 {
    database
        .serialized(|connection| {
            let count: i64 =
                connection.query_row("SELECT COUNT(*) FROM design_specs", [], |row| row.get(0))?;
            Ok(count)
        })
        .expect("count")
}

#[test]
fn separate_writer_connections_observe_busy_timeout() {
    let directory = tempfile::TempDir::new().expect("temp dir");
    let path = directory.path().join("contended.sqlite3");
    let first = Database::open(&path).expect("first");
    let second = Database::open_with_busy_timeout(&path, 50).expect("second");

    std::thread::scope(|scope| {
        let first = &first;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let writer_barrier = std::sync::Arc::clone(&barrier);
        let writer = scope.spawn(move || {
            first
                .transaction(|connection| {
                    connection.execute(
                        "INSERT INTO design_specs(id, model_json) VALUES ('spec-held', '{}')",
                        [],
                    )?;
                    writer_barrier.wait();
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    Ok(())
                })
                .expect("first writer commits");
        });
        barrier.wait();
        let contended = second.transaction(|connection| {
            connection.execute(
                "INSERT INTO design_specs(id, model_json) VALUES ('spec-blocked', '{}')",
                [],
            )?;
            Ok(())
        });
        assert!(
            matches!(contended, Err(PersistenceError::Sqlite(_))),
            "expected SQLITE_BUSY, got {contended:?}"
        );
        writer.join().expect("writer join");
    });

    second
        .transaction(|connection| {
            connection.execute(
                "INSERT INTO design_specs(id, model_json) VALUES ('spec-after', '{}')",
                [],
            )?;
            Ok(())
        })
        .expect("second writer succeeds after release");
    assert_eq!(
        second
            .serialized(|connection| {
                let count: i64 =
                    connection
                        .query_row("SELECT COUNT(*) FROM design_specs", [], |row| row.get(0))?;
                Ok(count)
            })
            .expect("count"),
        2
    );
}
