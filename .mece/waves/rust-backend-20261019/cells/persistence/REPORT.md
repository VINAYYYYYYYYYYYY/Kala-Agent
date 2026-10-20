# Persistence cell report — native SQLite schema, journal and snapshots

**Status: implementation complete for this cell's slice plus the round-2
review repairs; no commit, push, deployment, model change or delegation was
made, and no performance claim is offered.** The foundation's `kala-json` was
consumed unchanged. Native Windows execution is still unverified.

## 1. Ownership and files

Owned and created/updated:

- `backend/Cargo.toml` (workspace members now
  `["crates/kala-json", "crates/kala-persistence"]`; geometry/budget exclusions
  unchanged), `backend/Cargo.lock`
- `backend/README.md` (narrow workspace status update)
- `backend/crates/kala-persistence/**` (crate, sources, integration tests and
  the test-only Python cross-reader oracle)
- `.mece/waves/rust-backend-20261019/cells/persistence/REPORT.md`,
  `SESSION.md`

Read-only and untouched:

- `backend/fixtures/**` (verified by SHA-256 below), `backend/compat/**`
- `backend/crates/kala-json/**`, `crates/kala-geometry/**`,
  `crates/kala-budget/**`
- `src/**`, `tests/**`, `studio/**`, `adapters/**`, packaging, docs
- `Rust+buid123/**`, `rust-implementation.md`, `docs/ui-screens-wireframes.md`
- user-modified Python files already dirty before this cell
  (`src/kala_agent/__init__.py`, `api/{__init__,middleware,schemas,envelope}.py`,
  `tests/unit/test_{portable_imports,solidedge_boundaries}.py`) were not
  touched.

`git status` for this cell shows only the previously untracked `backend/`
subtree; no fixture or user-owned file was modified.

## 2. Deliverables

### Workspace

- Native `kala-persistence` member beside `kala-json`; `cargo metadata
  --no-deps` reports exactly two packages, both edition 2024, MSRV 1.85.
- One reproducible `Cargo.lock`; all commands below were run with `--locked`.
  New resolved versions: `rusqlite 0.40.2`, `libsqlite3-sys 0.38.2`
  (bundled SQLite 3.5x compiled from source), `chrono 0.4.45`,
  `parking_lot 0.12.5`/`parking_lot_core 0.9.12`, dev `tempfile 3.27.0`,
  `serde_json 1.0.x`.
- `backend/.gitignore` already ignores `target`; `Cargo.lock` retained.

### Public API (`backend/crates/kala-persistence/src/lib.rs`)

```rust
pub struct Database;                  // open / in_memory / open_with_config /
                                      // open_with_busy_timeout / initialize /
                                      // schema_version / schema_digest /
                                      // transaction / serialized /
                                      // with_connection / connection / close
pub struct DatabaseConfig;            // busy_timeout_ms, validated 0..=2^31-1
pub struct EventJournal<'a>;          // new / append / append_with /
                                      // append_value / get / read /
                                      // read_filtered / verify_chain
pub struct Event;                     // sequence, ids, payload_json, typed
                                      // digests, UTC timestamp, payload()
pub struct EventFilter;               // type/aggregate/workflow/run ids,
                                      // after_sequence, limit (default 1_000)
pub struct AppendOptions;             // workflow_id, run_id, occurred_at
pub struct ChainVerification;         // event_count, head_sequence,
                                      // head_event_digest
pub struct SnapshotStore<'a>;         // save / save_value / get / latest /
                                      // restore / restore_latest
pub struct WorkflowSnapshot;          // sequence, ids, state_json, typed
                                      // digest, UTC timestamp, state()
pub enum PersistenceError;            // typed context for later HTTP mapping
pub enum JournalIntegrityError;       // continuity/link/payload/digest/…
pub enum SchemaMigration / MigrationStep; pub static MIGRATIONS;
pub fn canonicalize_json_object(_bytes); pub fn parse_json_object(_bytes);
pub fn canonicalize_object_value; pub fn persistence_limits();
pub fn is_lowercase_sha256_hex; pub fn parse_stored_digest;
pub fn sha256_text; pub fn timestamp_to_iso; pub fn timestamp_from_iso;
pub fn truncate_to_micros;
```

`Database` holds one `rusqlite::Connection` behind a reentrant writer lock
(`parking_lot::ReentrantMutex<RefCell<Option<Connection>>>`), so exactly one
logical writer runs at a time and nested transactions become
`SAVEPOINT kala_agent_persistence`. Every transaction is wrapped in a guard:
operation errors, failed `COMMIT`/`RELEASE` and Rust panic unwinding all roll
back the live transaction or savepoint before the original error/panic leaves
`transaction`. A failed `COMMIT` cannot leave a hidden outer transaction for
the next caller to nest into. `close` is idempotent, access after close is
`PersistenceError::Closed`, and `Drop` closes. While a `ConnectionGuard` is
alive, `close` returns `ConnectionInUse` instead of removing the connection
from under the guard.

### Schema (`src/schema.rs`)

The frozen migration tuples are reproduced statement-for-statement from
`kala_agent/persistence/database.py`: migration 1
`initial_persistence_schema` (8 tables, indexes, the two append-only triggers)
and migration 2 `journal_event_hash_chain` (two `ALTER TABLE` columns, drop the
update trigger, the chain backfill callable, recreate the update trigger,
create the chained-insert trigger). Migration names/version ordering,
`BEGIN IMMEDIATE`, refusal of newer schemas, and atomic rollback on failure are
preserved. Backfill refuses non-contiguous or payload-digest-failing history
and never resets a database.

## 3. Compatibility behavior implemented

### Schema digest and logical rows

A freshly initialized native database produces the **exact** fixture
normalized `sqlite_master` digest
`a8b36de324421812e27eddcaa49487482b81e8293fbd4edb806b83dfbfb60b9a`
(same algorithm: sorted `[type,name,tbl_name,normalized sql]`, canonical JSON,
SHA-256), and reading the Python-created fixture through the native
`Database::schema_digest` returns the same value. Trigger names, table names,
column order, `schema_migrations` rows, WAL journal mode and
`PRAGMA foreign_keys = ON` match the fixture; no `synchronous` downgrade is
set, so SQLite's default FULL durability is kept.

### Journal

- Append allocates `tail.sequence + 1` under the writer lock, validates the
  verified tail (payload digest, raw lowercase chain strings, recomputed chain
  digest) and inserts with the exact eight covered parts and
  `kala-agent.event-chain.v1.sha256` domain.
- `EventJournal::new`, every `read`/`read_filtered` and every `get` walk the
  whole journal in sequence order and check continuity, raw lowercase 64-hex
  chain strings **before** `Sha256Digest` conversion, payload canonical bytes,
  payload digest, identifiers, timestamp validity, previous-digest linkage and
  the recomputed event digest. Filters never reduce the verification scope.
- Chain preimages use the **raw stored bytes** of `event_type`,
  `aggregate_id`, `workflow_id` and `run_id`. Identifier trimming happens only
  after verification, for the typed `Event` view, exactly like Python
  `chain_fields_from_row` plus `_event_from_row`. Whitespace padding without a
  rehash is therefore detected; a padded row whose digest genuinely covers the
  padding is accepted and yields the trimmed API identifier. The `occurred_at`
  string is likewise hashed from raw stored bytes and parsed separately.
- Stored payloads/digests/ids/timestamps are re-checked when a row is
  converted; uppercase stored chain strings and malformed digests are refused.
- Snapshot save/get/latest/restore/restore_latest verify canonical bytes and
  the stored digest on every read; duplicate ids fail with the SQLite unique
  violation.

### Canonical JSON domain

`persistence_limits()` never uses the tool defaults: `max_json_bytes`,
`max_string_length`, `max_collection_items` and `max_key_length` are
effectively unbounded, and only syntactic depth is bounded at 1,024 levels
(recursive-descent safety). The 4,300-digit CPython integer limit is kept. The
native parser/serializer is the same `kala-json` foundation, so Unicode
(code points, lone surrogates, supplementary escapes), `-0.0`, `1e-05`,
arbitrary-precision integers and CPython sorted-key ASCII bytes are preserved.

### Timestamps

`timestamp_to_iso` emits UTC `YYYY-MM-DDTHH:MM:SS.ffffffZ` exactly like
`_timestamp_to_iso`, accepts any `chrono` timezone, truncates nanoseconds to
Python's microseconds, enforces years `1..=9999`, and refuses a `chrono`
leap-second value (`23:59:60`/`:60`), which Python `datetime` cannot represent.
`timestamp_from_iso` mirrors live `datetime.fromisoformat` (C implementation)
over `value.replace("Z","+00:00")`: extended/basic calendar dates, extended and
basic ISO-week dates, `HH`, `HH:MM`, `HH:MM:SS` and basic equivalents, an
arbitrary single-character date/time separator, dot or comma fractions with
microsecond truncation, and `±HH`, `±HHMM`, `±HH:MM`, `±HHMMSS`,
`±HH:MM:SS` offsets with optional seconds/fractions. It requires an offset,
rejects leap seconds, enforces Python's strict `(-24h, 24h)` offset range, and
rejects the `9999-12-31T24:00` local rollover exactly as Python does.
`Utc::now()` and all explicit inputs are truncated to microseconds before
storage.

## 4. Fixture, differential and cross-reader evidence

Consumed read-only:

- `backend/fixtures/corpus/v1/schema_v2.sqlite3` — copied to a temporary
  directory for every open; source SHA-256 remains
  `9d8b6556c99006ce287a52c65e2590ef5d2b485ec73bdd4079fb648191025832`.
- `backend/fixtures/corpus/v1/event_chain.json` — SHA-256 remains
  `4cc4dc39e3900ce0b22aa58230ed05e66290351b9be0005f0e0d51ecd93420c7`.

Compared byte-for-byte against the fixture:

- all 5 events (sequence, ids, `payload_json`, payload digest, `occurred_at`,
  previous digest, event digest), verified count 5 and head
  `143fed75e63b8fb67c9449b0fe353cf4d20fc1cb4223b20e5d70e95028999fc7`;
- the snapshot row (`state_json`, digest, created_at), `latest`/`restore`
  equality and a close/reopen cycle;
- normalized schema digest and migration rows for a freshly created database.

Live Python oracle (`CPython 3.14.8`, `.venv/bin/python`,
`src/kala_agent` imported from this checkout):

- `tests/canonical_domain.rs` compares 20 object sources against
  `contracts.models._canonical_json_object`: 33,000-code-point string,
  300-code-point key, 2,100-item array, 2,100-entry object, depth 49/64/200/
  500/900, 1.1 MiB payload, lone surrogate, 4,300-digit integer, plus duplicate/
  non-finite/top-level/4,301-digit refusals. 13 sources are accepted by both
  with byte-equal, reparse-stable output; the other 7 are refused by both.
- `tests/python_cross_reader.rs` opens, verifies and extends databases in both
  directions:
  - Rust creates a database, appends a payload beyond every tool bound and a
    snapshot; **Python** `Database`/`EventJournal`/`SnapshotStore` reopen it,
    verify all bytes/head, then append a large-domain payload; Rust reopens and
    verifies Python's head/digests.
  - **Python** creates a database with fixed-clock events and a snapshot; Rust
    verifies, compares every row, appends and snapshots; Python reopens the
    Rust-extended database and verifies count/head/payload/snapshot.
  - A copied corpus fixture is verified/read by Rust, extended with an event
    and snapshot, then fully verified by Python against the recorded fixture
    rows and the new head.
- Tamper corpus (`tests/journal_security.rs`) drops the triggers as an
  attacker would and requires detection for: sequence continuity, every
  covered field, payload without digest, invalid/zero/uppercase/malformed
  chain digests, uppercase payload digest, omitted event, adjacent reorder,
  rechain-with-noncanonical payload, invalid identifiers/timestamps,
  tamper-while-closed startup refusal, tampered-head append refusal and
  rewritten-history head change. Filters that match nothing still fail.
  Round 2 adds whitespace padding of each chain-covered identifier on a middle
  row (startup/read/get/filter refusal) and on the head (append refusal), plus
  a correctly rehashed padded row that Rust and Python both verify and expose
  with trimmed identifiers.
- Round 2 timestamp differential (`tests/timestamp_compat.rs`) sends every
  case to the live `_timestamp_from_iso`/`_timestamp_to_iso` oracle. Besides
  the reviewer matrix it covers calendar/week/basic dates, hour/minute/second
  forms, separator variants, comma fractions, offset seconds/fractions,
  leap-second refusal, 24:00 rollover, year boundaries, invalid dates/offsets
  and an 8,160-case generated form matrix; Rust accepted/rejected and UTC
  microsecond output match Python in every case. A stored-event test rehashes
  a 9-digit fractional timestamp and a week-date timestamp with offset seconds,
  then checks Rust and Python return identical typed values.
- Transaction regression tests (`tests/schema_migration.rs`) cover a
  deferred-FK `COMMIT` failure followed by a real successful transaction,
  failed nested `RELEASE` preserving the outer transaction, outer-level panic
  unwind followed by reuse, nested panic rolling back only the savepoint, and
  `close` refusal while a guarded connection is held.

## 5. Checks actually run

Host: Linux x86_64, `cargo/rustc 1.99.1`, oracle
`backend/.venv/bin/python` (CPython 3.14.8). All from `backend/`:

| Command | Result |
|---|---|
| `cargo fmt --all --check` | exit 0 |
| `cargo test --workspace --locked` | exit 0; **76 tests**, 0 failed/ignored (kala-json 17+3+3; kala-persistence 9 unit + 5 canonical + 4 fixture + 13 security + 3 cross-reader + 16 schema + 3 timestamp) |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| `cargo build --release --workspace --locked` | exit 0 (`libkala_persistence.rlib`) |
| `cargo metadata --no-deps` | exactly `kala-json`, `kala-persistence`; geometry/budget excluded |

The release build is compilation evidence only. No benchmark, throughput,
RSS or speedup was measured or is claimed.

## 6. Round-2 repair summary

Reproduced and fixed all three review findings; no finding was waived.

1. **P1 raw identifier bytes.** `validate_event_row`/`chain_fields` now keep
   and hash the raw stored identifier strings. Normalization moved to
   `event_from_row` only, so `verify_chain`, `get`, reads, filters and the
   verified tail commit to stored bytes while the typed `Event` API still
   returns Python's trimmed identifiers. Regression coverage: padding each of
   the four identifiers on a middle row and on the head, startup refusal,
   filter-with-no-match refusal, append-head refusal, and a raw-rehashed padded
   row that Python also accepts.
2. **P1 failed finalization / panic.** `Database::transaction` now uses a
   `TransactionState` guard. `COMMIT`/`RELEASE` errors trigger rollback before
   the error is returned, operation errors roll back their own level, and
   `Drop` performs the same rollback during panic unwinding. Regression
   coverage: deferred-FK `COMMIT` failure (autocommit and row count checked,
   then a real transaction succeeds), failed nested `RELEASE` preserving the
   outer transaction, outer error rolling back already-released savepoint work,
   outer panic then reuse, nested panic rolling back only the nested row.
3. **P2 timestamp domain.** The reader now mirrors CPython C
   `datetime.fromisoformat`: week/basic dates, arbitrary single-character
   separator, `HH`/`HH:MM`/`HH:MM:SS` forms, comma fractions, offset
   seconds/fractions, 24:00 rollover and Python's local-year rollover refusal.
   Sub-microsecond digits are truncated for the typed value; `occurred_at`
   remains hashed raw. Writers truncate to microseconds, reject leap seconds
   and enforce the year/offset domains. The live differential includes the
   reviewer matrix, boundaries and an 8,160-case generated matrix.
4. **ConnectionGuard lifecycle.** `connection()` counts live guards and
   `close()` refuses with `ConnectionInUse` while any guard exists; the guard
   remains usable and a later close succeeds after drop. This removes the
   stale-guard panic path.

## 7. Gaps, divergences and unresolved scope

1. **Windows.** No Windows target/toolchain is installed on this host, so no
   MSVC/gnu build, `.exe` or COM proof exists. Source-level portability is
   limited to `#[cfg(unix)]` permission code (`std::os::unix` is never
   imported unguarded) and bundled SQLite; compilation on Windows is
   unverified and reported rather than assumed.
2. **Deferred scope.** Typed domain-model repositories (design/revision/
   approval/tool/memory), artifact CAS, provenance policy, HTTP/core mapping,
   CLI and packaging are intentionally not implemented here. `Decimal`
   persistence belongs to the budget cell; raw `Decimal`/non-string keys remain
   outside the typed canonical value domain, as in the foundation.
3. **Depth boundary (foundation follow-up).** The native parser accepts at most
   1,023 nested arrays inside a root object (constant depth 1,024); deeper
   `CanonicalJsonObject` input fails closed with `CanonicalJsonError::NotJson`.
   CPython 3.14 in the corpus process accepted 52,120 nested arrays before its
   recursive decoder failed, so that boundary is process/stack-dependent and
   the accepted/rejected sets are **not** identical for very deep stored data.
   This is a foundation-owned gap requiring an iterative/nonrecursive path; it
   was not worked around by editing `kala-json`, narrowing source data, or
   resetting rows in this cell. Rows deeper than the native bound fail closed
   and are left untouched; reading them requires the separately coordinated
   foundation fix.
4. **Append verification.** Appends validate the verified tail exactly like the
   Python source (reads and `get` always verify the whole chain). A middle-row
   tamper followed by an append is caught by the next read, not by the append
   itself; this preserves source behavior and is stated here explicitly.
5. **Close/in-use lifecycle.** `close` now returns
   `PersistenceError::ConnectionInUse` while any `ConnectionGuard` exists, so a
   held guard cannot outlive the connection and later panic in
   `ConnectionGuard::with`. A reentrant `initialize` inside `transaction`
   likewise returns `ConnectionInUse` if a mutable borrow is active (Python
   nests a savepoint instead); normal lifecycle usage is unaffected. The guard
   refusal has a regression test.
6. **Multi-violation error precedence** for a single canonical JSON document
   follows the foundation's sorted-key traversal instead of Python's insertion
   order; the accepted/rejected set is identical.
7. **Error mapping.** `PersistenceError`/`JournalIntegrityError` carry the
   context needed for later core HTTP status mapping, but no route or envelope
   mapping is implemented in this cell.
8. **Timestamp scope.** The reader matches live CPython C
   `datetime.fromisoformat` behavior for the reviewer matrix and an 8,160-case
   generated form matrix, including its documented and observed leniencies.
   A lone-surrogate date/time separator accepted by CPython's in-memory parser
   cannot exist in Rust `&str` or SQLite UTF-8 text, so no claim is made for
   that unrepresentable input; stored rows cannot contain it.
9. **No commit/push/deploy** was performed and no user-owned file changed.

## 8. Next

The persistence contract is ready for the core service/HTTP vertical to
consume after reviewer acceptance: open `Database`, build `EventJournal`, and
map `PersistenceError` variants to the existing core envelope. The full
backend rewrite continues after this slice.
