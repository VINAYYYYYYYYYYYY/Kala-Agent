# kala-repositories — cell report

## Outcome

Ported the complete Python repository layer to a new standalone Rust crate:
`backend/crates/kala-repositories/`. All six repositories and the validated
immutable `MemoryRecord`, `HeadMovedError` and `ApprovalAlreadyDecidedError`
are implemented over the accepted `kala-persistence` `Database` and validated
through the accepted `kala-contracts` models. No schema, connection pool,
transaction manager or database abstraction was duplicated.

The crate is a standalone workspace with its own `Cargo.lock`. No root
manifest/lock, accepted crate, Python product/test, fixture, adapter,
packaging/desktop/frontend or other-wave file was edited by this cell.

## Owned changed paths

New files only:

```text
backend/crates/kala-repositories/.gitignore
backend/crates/kala-repositories/Cargo.toml
backend/crates/kala-repositories/Cargo.lock
backend/crates/kala-repositories/README.md
backend/crates/kala-repositories/src/lib.rs
backend/crates/kala-repositories/src/error.rs
backend/crates/kala-repositories/src/memory.rs
backend/crates/kala-repositories/src/repositories.rs
backend/crates/kala-repositories/src/support.rs
backend/crates/kala-repositories/tests/support/mod.rs
backend/crates/kala-repositories/tests/oracle/repositories_oracle.py
backend/crates/kala-repositories/tests/round_trip.rs
backend/crates/kala-repositories/tests/memory_record.rs
backend/crates/kala-repositories/tests/cas_concurrency.rs
backend/crates/kala-repositories/tests/schema_integrity.rs
backend/crates/kala-repositories/tests/cross_reader.rs
.mece/waves/rust-backend-20261019/cells/repositories/REPORT.md
.mece/waves/rust-backend-20261019/cells/repositories/SESSION.md
```

## Public API

Re-exported accepted types: `Database`, `EventJournal`, `PersistenceError`,
`DesignSpec`, `DesignRevision`, `ApprovalRequest`, `ToolCall`, `ToolResult`,
`Timestamp`, `Text`, `Identifier`, `CanonicalJsonObject`, `CanonicalValue`,
`ContractModel`, `ContractError`, `ContractIssue`, `ApprovalKind`,
`ApprovalStatus`, `SideEffectLevel`, `ToolRunStatus`, `NonNegativeInteger`,
`Sha256Digest`, `BigInt`, `Limits`.

### Constants / helpers

```rust
pub const DEFAULT_LIST_LIMIT: i64 = 1_000;
pub const MEMORY_RECORD: &str = "MemoryRecord";
pub const fn memory_metadata_limits() -> Limits;
```

### `DesignSpecRepository<'a>`

```rust
pub fn new(database: &'a Database) -> Self;
pub fn save(&self, spec: &DesignSpec) -> Result<(), RepositoryError>;
pub fn get(&self, spec_id: &str) -> Result<Option<DesignSpec>, RepositoryError>;
pub fn list(&self) -> Result<Vec<DesignSpec>, RepositoryError>;
pub fn list_with_limit(&self, limit: i64) -> Result<Vec<DesignSpec>, RepositoryError>;
```

### `DesignRevisionRepository<'a>`

```rust
pub fn new(database: &'a Database) -> Self;
pub fn commit_revision(
    &self,
    revision: &DesignRevision,
    expected_head_revision_id: Option<&str>,
) -> Result<DesignRevision, RepositoryError>;
pub fn get(&self, revision_id: &str) -> Result<Option<DesignRevision>, RepositoryError>;
pub fn list_for_design(&self, design_id: &str) -> Result<Vec<DesignRevision>, RepositoryError>;
pub fn list_for_design_with_limit(
    &self,
    design_id: &str,
    limit: i64,
) -> Result<Vec<DesignRevision>, RepositoryError>;
pub fn latest_for_design(
    &self,
    design_id: &str,
) -> Result<Option<DesignRevision>, RepositoryError>;
```

No `save`/`replace`/`upsert` exists on the revision repository.

### `ApprovalRequestRepository<'a>`

```rust
pub fn new(database: &'a Database) -> Self;
pub fn save(&self, request: &ApprovalRequest) -> Result<(), RepositoryError>;
pub fn get(&self, request_id: &str) -> Result<Option<ApprovalRequest>, RepositoryError>;
pub fn list_for_workflow(
    &self,
    workflow_id: &str,
) -> Result<Vec<ApprovalRequest>, RepositoryError>;
pub fn list_for_workflow_with_limit(
    &self,
    workflow_id: &str,
    limit: i64,
) -> Result<Vec<ApprovalRequest>, RepositoryError>;
```

### `ToolCallRepository<'a>`

```rust
pub fn new(database: &'a Database) -> Self;
pub fn save(&self, call: &ToolCall) -> Result<(), RepositoryError>;
pub fn get(&self, call_id: &str) -> Result<Option<ToolCall>, RepositoryError>;
pub fn list_for_tool(&self, tool_id: &str) -> Result<Vec<ToolCall>, RepositoryError>;
pub fn list_for_tool_with_limit(
    &self,
    tool_id: &str,
    limit: i64,
) -> Result<Vec<ToolCall>, RepositoryError>;
```

### `ToolResultRepository<'a>`

```rust
pub fn new(database: &'a Database) -> Self;
pub fn save(&self, result: &ToolResult) -> Result<(), RepositoryError>;
pub fn get(&self, call_id: &str) -> Result<Option<ToolResult>, RepositoryError>;
```

### `MemoryGetOptions` / `MemoryListOptions`

```rust
pub struct MemoryGetOptions {
    pub now: Option<Timestamp>,
    pub include_expired: bool,
}
pub fn with_now(self, now: Timestamp) -> Self;
pub fn with_include_expired(self, include_expired: bool) -> Self;

pub struct MemoryListOptions {
    pub source: Option<String>,
    pub now: Option<Timestamp>,
    pub include_expired: bool,
    pub limit: i64,
}
pub fn with_source(self, source: impl Into<String>) -> Self;
pub fn with_now(self, now: Timestamp) -> Self;
pub fn with_include_expired(self, include_expired: bool) -> Self;
pub fn with_limit(self, limit: i64) -> Self;
```

Defaults: `now = None` (current instant), `include_expired = false`,
`limit = 1_000`.

### `MemoryRepository<'a>`

```rust
pub fn new(database: &'a Database) -> Self;
pub fn put(&self, record: &MemoryRecord) -> Result<(), RepositoryError>;
pub fn get(&self, memory_id: &str) -> Result<Option<MemoryRecord>, RepositoryError>;
pub fn get_with_options(
    &self,
    memory_id: &str,
    options: &MemoryGetOptions,
) -> Result<Option<MemoryRecord>, RepositoryError>;
pub fn list(&self) -> Result<Vec<MemoryRecord>, RepositoryError>;
pub fn list_with_options(
    &self,
    options: &MemoryListOptions,
) -> Result<Vec<MemoryRecord>, RepositoryError>;
pub fn delete(&self, memory_id: &str) -> Result<bool, RepositoryError>;
```

### `MemoryRecord`

```rust
pub struct MemoryRecord { /* private validated fields */ }

pub fn new(
    memory_id: &str,
    content: impl Into<Text>,
    source: &str,
    provenance_id: Option<&str>,
    metadata_json: CanonicalJsonObject,
    created_at: Timestamp,
    expires_at: Option<Timestamp>,
) -> Result<Self, ContractError>;

pub fn memory_id(&self) -> &str;
pub fn content(&self) -> &Text;
pub fn source(&self) -> &str;
pub fn provenance_id(&self) -> Option<&str>;
pub fn metadata_json(&self) -> &CanonicalJsonObject;
pub fn created_at(&self) -> &Timestamp;
pub fn expires_at(&self) -> Option<&Timestamp>;

impl ContractModel for MemoryRecord { ... }
// from_json_str / from_json_bytes / to_wire_json / to_wire_json_ascii
```

`MemoryRecord` derives `Clone, Debug, PartialEq, Eq`; all fields are private
and there are no mutators (frozen equivalent).

### Errors

```rust
pub struct HeadMovedError {
    pub message: String,
    pub design_id: String,
    pub expected_head_revision_id: Option<String>,
    pub actual_head_revision_id: Option<String>,
    pub expected_sequence: BigInt,
    pub actual_sequence: BigInt,
}
// + design_id(), expected_head_revision_id(), actual_head_revision_id(),
//   expected_sequence(), actual_sequence()

pub struct ApprovalAlreadyDecidedError { pub request_id: String }
// + request_id()

pub enum RepositoryError {
    InvalidIdentifier { field: &'static str },
    InvalidLimit,
    Validation(ContractError),
    StoredModel { model: &'static str, source: ContractError },
    ToolResultMissingCall,
    ToolResultToolMismatch,
    SequenceOutOfRange,
    TimestampOutOfRange,
    HeadMoved(Box<HeadMovedError>),
    ApprovalAlreadyDecided(ApprovalAlreadyDecidedError),
    Persistence(PersistenceError),
}
// + as_head_moved, as_approval_already_decided, as_persistence, as_sqlite
```

## Behavior preserved

- `DesignSpec`: unconditional insert-or-replace; `ORDER BY id`; default limit
  1000.
- `DesignRevision`: append-only CAS only; required expected head; head read,
  sequence check, spec `ON CONFLICT DO NOTHING`, and revision insert in one
  `BEGIN IMMEDIATE` (or nested `SAVEPOINT`); `ORDER BY sequence ASC`;
  `latest` is `DESC LIMIT 1`. `HeadMovedError` field values and message text
  match Python, including the sequence-mismatch variant.
- `ApprovalRequest`: conditional UPSERT exactly as Python; decided rows only
  accept a byte-identical `model_json` no-op, and that path still executes the
  SQL UPDATE; pending rows may change; `ORDER BY created_at ASC, request_id ASC`.
- `ToolCall`: insert-or-replace; `ORDER BY issued_at ASC, call_id ASC`.
- `ToolResult`: call existence and `tool_id` identity checked in the same
  transaction as the UPSERT; schema FK remains.
- `Memory`: source/provenance/metadata validation; strict
  `expires_at > now`; `ORDER BY created_at ASC, memory_id ASC`; delete returns
  `rowcount == 1`.
- Every lookup normalizes identifiers with Python `str.strip()` semantics and
  validates positive limits.
- All stored `model_json` is decoded with accepted contract validation.
  Pydantic-style duplicate-key last-wins applies to model JSON; canonical
  metadata remains strict about duplicates.
- Writers generate Pydantic `model_dump_json()` bytes. Raw legacy bytes are
  never rewritten on read.

## Verification

All commands run in `backend/crates/kala-repositories`.

| Command | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo test --locked` (debug) | 33 passed, 0 failed, 0 ignored |
| `cargo test --locked --release` | 33 passed, 0 failed, 0 ignored |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo build --release --locked` | clean |
| live oracle | `KALA_REPOSITORIES_PYTHON=.venv/bin/python cargo test --locked --test cross_reader` → 3 passed |
| Python reference regression | `pytest tests/unit/test_persistence.py` → 16 passed |
| Python revision security regression | `pytest tests/security/test_revision_commit.py` → 26 passed |

Test distribution: `cas_concurrency` 7, `cross_reader` 3, `memory_record` 7,
`round_trip` 8, `schema_integrity` 8.

Coverage highlights:

- real SQLite files, reopen, all six repository round trips, update,
  expiry, filtering, limit boundaries and delete;
- live Python seed → Rust read and Rust seed → Python inspect; every model
  re-serialized by Python exactly matches raw stored `model_json`
  (`raw_round_trip` 9/9 for the Python seed, 8/8 for the Rust seed);
- raw `model_json` and UTC microsecond timestamp columns compared in both
  directions, including non-UTC offsets preserved in model JSON;
- original frozen fixture `schema_v2.sqlite3` opened and read after copying;
- exact approval finality and no-op SQL behavior, missing/mismatched tool
  results, schema FK/UNIQUE/append-only trigger rejections;
- parent/nested transaction rollback, failed-COMMIT rollback, post-failure
  autocommit and successful reuse;
- two independent `Database` connections racing revision CAS and approval
  decisions; one winner, typed loser error, no orphan spec/revision, no fork,
  both connections usable afterwards;
- tampered stored models for all six tables fail with typed `StoredModel`
  errors and are not rewritten;
- 4,000-digit metadata integers, 80-level metadata, >1 MiB metadata in both
  directions, a 1.1 MiB `DesignSpec.attributes_json`, duplicate-key
  distinctions and surrogate escapes;
- Python-style identifier whitespace normalization, max/overlong IDs, and
  arbitrary `sequence` reporting with the documented SQLite-range boundary.

## Concurrency / cross-reader evidence

- CAS race: `concurrent_cas_writers_on_independent_connections_produce_one_winner`.
  Two connections, barrier-synchronized, both prepared at the empty head. One
  `commit_revision(seq 0, expected None)` succeeds; the loser receives
  `HeadMoved` with `expected_sequence = 0`, `actual_sequence = 0`, and the
  winner's head. Exactly one revision and the winner's spec exist; the loser's
  spec does not. Both database connections report autocommit, and a subsequent
  append succeeds.
- Approval race: two connections decide the same pending request concurrently.
  One final row remains; the loser receives `ApprovalAlreadyDecidedError`; a
  reopen attempt also fails; the connection then writes memory successfully.
- Cross-reader wire evidence: raw `model_json` equality for spec, revisions,
  approval, call, result and memory; Python `model_dump_json()` equals Rust
  `to_wire_json()` and the stored bytes; timestamps stored as
  `YYYY-MM-DDTHH:MM:SS.ffffffZ` while offsets remain in model JSON.

## Dependency API gaps (reported, accepted sources not edited)

1. `kala_persistence::Database::transaction` only accepts
   `Result<T, PersistenceError>`. A repository needs `HeadMovedError` /
   `ApprovalAlreadyDecidedError` to escape the same rollback path. The crate
   records the real error beside the closure and aborts with a private
   `PersistenceError` sentinel so the accepted transaction guard still performs
   normal rollback (including savepoint and failed-COMMIT cleanup). A generic
   `transaction_with<T, E>(...) where E: From<PersistenceError>` (or an
   error-erasing guard) would remove the shim.
2. `kala_contracts::Timestamp` has no direct conversion to/from
   `chrono::DateTime<Utc>`, which `kala_persistence::timestamp_to_iso` takes.
   The bridge uses `Timestamp::absolute_micros()` and
   `DateTime::from_timestamp_micros`; values are inside the Python year
   1..=9999 domain, so the conversion is lossless for validated values.
3. `kala_contracts` exposes validated models only through
   `ContractModel::from_value`/JSON entry points; there are no public typed
   builders. Future kernel/application consumers must build model JSON or
   `CanonicalValue` inputs. This is usable but worth deciding before the next
   layer.

No accepted source was modified to address these.

## Explicit limitations

- Typed API differences: Python's `TypeError` for a non-`DesignSpec` /
  non-integer (including `bool`) limit cannot occur through Rust
  references/`i64`; non-positive limits still fail identically at runtime.
- `limit` is `i64`; Python arbitrary-precision limits outside that range are
  not representable.
- A revision `sequence` matching the head at SQLite's `i64::MAX` returns
  `SequenceOutOfRange`; Python's `sqlite3` raises `OverflowError` at insert.
- Stored model validation failures are typed `StoredModel` errors rather than
  Pydantic `ValidationError`; the model name and accepted `ContractError` are
  preserved. Non-TEXT values in `model_json`/column fields surface as SQLite
  column-type errors rather than Python's `str(row)` coercion; normal writers
  always store TEXT.
- No kernel/application/HTTP/CLI integration and no second DB abstraction were
  added; this cell is the repository layer only.
- Windows: `cargo check --locked --target x86_64-pc-windows-msvc --all-targets`
  stops in bundled SQLite (`cc-rs: failed to find tool "lib.exe"`). The
  installed MSVC target is present, but no MSVC linker/`lib.exe` exists on this
  host. No Windows compile or runtime claim is made.
- No performance benchmark or speedup claim was made.
- No commit, push, publish, deployment, cutover or Python deletion was
  performed.
- Oracle note: an early read-only `sqlite3` inventory query of
  `backend/fixtures/corpus/v1/schema_v2.sqlite3` checkpointed/removed its
  transient `-wal`/`-shm` sidecars (the main file mtime remained
  `2026-10-19 15:57`). No fixture rows were edited; tests copy the database and any
  `-wal`/`-shm` sidecars before opening the copy.

## Repair rounds

No conductor-requested repair round was needed. Development-time failures were
fixed while building the tests (initial unborn-sequence comparison and a test
format-string escape); they were not separate repair rounds.
