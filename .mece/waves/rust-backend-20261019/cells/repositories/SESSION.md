# kala-repositories — session log

## Scope and instructions read

- Read `WAVE.md`, `SYNTHESIS.md`, this cell's `TASK.md`/`LAUNCH.md`, the root
  `backend/README.md`, `src/kala_agent/persistence/repositories.py`,
  `database.py`, `contracts/models.py`, `tests/unit/test_persistence.py`,
  `tests/security/test_revision_commit.py`, and the accepted
  `kala-persistence`, `kala-contracts`, `kala-json` implementations/tests.
- Ownership: only new `backend/crates/kala-repositories/**` plus this cell's
  `REPORT.md`/`SESSION.md`.
- Standalone `[workspace]` with its own `Cargo.lock`; root manifests/locks and
  accepted crates untouched.
- No delegation, model/provider switch, commit, push, publishing, deployment,
  source deletion/reset, unrelated installation or cutover.
- Python remains the reference/oracle; production library has no Python or
  subprocess calls.

## Implementation decisions

1. **Shared accepted Database.** Every repository is a `Copy` borrow of
   `&kala_persistence::Database`. No schema, migration, connection pool or
   second lock manager exists in this crate.
2. **Accepted transaction semantics with typed errors.** The accepted
   `Database::transaction` closure only returns `PersistenceError`. The
   repository layer records the real typed error in a local slot and aborts the
   accepted transaction with a private sentinel, so the accepted guard performs
   rollback (including nested savepoints and failed COMMIT) and the recorded
   error is returned. The sentinel never escapes. Dependency gap recorded in
   `REPORT.md`.
3. **Contract validation on every read.** `model_json` is decoded through
   `ContractModel::from_json_str`; reads never rewrite stored bytes.
4. **Wire bytes.** Writes use `ContractModel::to_wire_json()` and UTC
   microsecond columns use `kala_persistence::timestamp_to_iso`. Live
   cross-readers confirm byte equality with Python `model_dump_json()`.
5. **MemoryRecord.** Implemented in this crate (it lives in the Python
   repository module, not `kala.contracts`), implementing the accepted
   `ContractModel` trait so it inherits the same Pydantic-compatible model JSON
   parser. Fields are private/immutable; `new` and `from_value` enforce
   `NonEmptyText` trimming/length/surrogate rules, canonical metadata, and the
   strict absolute-instant expiry comparison.
6. **Revision CAS.** `commit_revision` is the only write path: head read,
   expected-head comparison, sequence validation, spec
   `ON CONFLICT DO NOTHING`, revision insert in one immediate/savepoint
   transaction. `HeadMovedError` carries all Python fields; sequences are
   arbitrary-precision in the error.
7. **Approval finality.** Exactly the Python conditional UPSERT. A
   byte-identical no-op still runs the SQL UPDATE; tests tamper side columns and
   verify repair. Changed/reopened decided approvals raise the typed error.
8. **Tool result identity.** Call existence and `tool_id` match are checked and
   the UPSERT executed in one transaction; FK remains the backstop.
9. **Memory expiry.** Strict `expires_at > now` text comparison using the same
   UTC microsecond format; inclusive flag returns expired rows; source filter
   and `created_at, memory_id` ordering mirror Python.

## Work sequence

1. Inventoried Python repositories, database transaction/timestamp helpers,
   unit/security tests, schema migrations/triggers, accepted Rust DB and
   contract APIs.
2. Confirmed Pydantic raw transport examples (`model_dump_json`) and Pydantic
   metadata/duplicate/expiry behavior with the live `.venv` oracle.
3. Created the standalone crate and implemented `RepositoryError`,
   `HeadMovedError`, `ApprovalAlreadyDecidedError`, shared helpers,
   `MemoryRecord`, all six repositories, re-exports and README.
4. Wrote tests:
   - `round_trip.rs` — real-file round trips, updates, defaults/limits/tie
     breakers, identifier normalization/boundaries, expiry/filter/delete,
     absent lookups/default-now paths.
   - `memory_record.rs` — defaults, extra/required fields, content boundaries,
     C0 separator preservation, surrogate rejection, absolute expiry,
     canonical/duplicate rules, 4,000-digit integers, 80-level and >1 MiB
     metadata, and a >1 MiB DesignSpec attributes object.
   - `cas_concurrency.rs` — all Python `HeadMovedError` fields, rollback of
     spec+revision, unique constraints, parent/nested/failed-COMMIT rollback,
     post-failure usability, independent-connection CAS and approval races,
     arbitrary/overflow sequence handling.
   - `schema_integrity.rs` — schema tables/triggers/FK, append-only trigger,
     direct FK/UNIQUE rejections, approval finality/no-op SQL, atomic tool
     result identity, tampered stored models, duplicate-key model parsing,
     timestamp column normalization, frozen fixture readability.
   - `cross_reader.rs` + `tests/oracle/repositories_oracle.py` — live Python
     seed → Rust read, Rust seed → Python read/re-serialize, raw byte/timestamp
     comparison, deep/large metadata both directions.
5. Ran format, debug/release tests, all-target/all-feature clippy with
   `-D warnings`, release build, live oracle tests and the Python reference
   persistence/security tests.
6. Attempted the Windows MSVC all-target check; it stops in bundled SQLite
   because no MSVC `lib.exe` is installed. No Windows claim.
7. Wrote `REPORT.md` and this session log.

## Final verification results

- `cargo fmt --all --check`: clean.
- `cargo test --locked`: 33 passed, 0 failed, 0 ignored.
- `cargo test --locked --release`: 33 passed, 0 failed, 0 ignored.
- `cargo clippy --all-targets --all-features --locked -- -D warnings`: clean.
- `cargo build --release --locked`: clean.
- `KALA_REPOSITORIES_PYTHON=.venv/bin/python cargo test --locked --test cross_reader`:
  3 passed.
- `pytest tests/unit/test_persistence.py`: 16 passed.
- `pytest tests/security/test_revision_commit.py`: 26 passed.
- Windows MSVC cross-check: blocked by missing `lib.exe` in bundled SQLite
  build. No runtime claim.

## Notes / limitations

- Dependency API gaps (transaction error type, Timestamp/chrono bridge, no
  public model builders) are documented in `REPORT.md`; accepted sources were
  not edited.
- Explicit behavior boundaries: typed `i64` limit, `SequenceOutOfRange` at the
  SQLite integer ceiling, typed `StoredModel` instead of Pydantic
  `ValidationError`, non-TEXT stored column values surfaced as SQLite type
  errors.
- Early read-only `sqlite3` inventory of the corpus fixture checkpointed away
  its transient `-wal`/`-shm` sidecars; the main fixture was not row-edited and
  tests copy it (and any sidecars) before opening. Recorded in `REPORT.md`.
- No commit/push/deploy/cutover, no Python product or fixture edits, no root
  manifest/lock changes.
