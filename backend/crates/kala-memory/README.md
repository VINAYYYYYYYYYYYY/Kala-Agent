# kala-memory

Native working and durable memory management for the Kala Agent kernel. This
crate is the Rust counterpart of `kala_agent.kernel.memory`: it owns the
bounded working-memory layer that sits in front of the accepted durable store
and enforces "one content per source" across both layers.

It is a standalone workspace with its own `Cargo.lock`; it does not modify the
root backend workspace, any accepted crate, or any Python source or test.

## Accepted dependencies (read-only)

- `kala-repositories` — `MemoryRepository`, `MemoryRecord`, the repository
  error type and the durable read/write semantics.
- `kala-contracts` — `Timestamp`, `Text`, `CanonicalJsonObject`,
  `ContractModel` and the contract error type.
- `kala-json` — the canonical JSON value model used by the result projections.
- `kala-persistence` — the `Database` the repository borrows.

No schema, migration, connection pool or second database abstraction was
duplicated. The manager owns no storage: it reads and writes through the
accepted `MemoryRepository`.

## Public API

```rust
use kala_memory::{
    AddOptions, GetOptions, ListOptions, ListWorkingOptions, MemoryManager,
    MemoryManagerConfig, PromoteOptions, PromoteTarget,
};
use kala_persistence::Database;
use kala_repositories::MemoryRepository;

let database = Database::open("state/kala.sqlite3")?;
let manager = MemoryManager::new(MemoryRepository::new(&database));
```

### `MemoryManager<'a>`

| Method | Python counterpart | Behavior |
|---|---|---|
| `new(repository)` | `MemoryManager(repository)` | default configuration |
| `with_config(repository, config)` | `MemoryManager(repository, max_working_records=..., require_provenance_on_promotion=...)` | refuses a capacity below one with `"max_working_records must be positive"` |
| `repository()` | `repository` | the accepted durable repository |
| `max_working_records()` | `max_working_records` | the working-map capacity |
| `require_provenance_on_promotion()` | — | the configuration flag |
| `add(record, options)` | `add` / `add_record` | working add, optional promotion; a conflict raises `MemoryError::Conflict` |
| `try_add(record, now)` | `try_add` | working add that reports `added`/`duplicate`/`conflict` |
| `promote(target, options)` | `promote` / `promote_record` | durable promotion of a record or an identifier |
| `get(memory_id, options)` | `get` | working first, then the durable repository |
| `list(options)` | `list` | active working and durable records, ordered by `(created_at, memory_id)` |
| `list_active(options)` | `list_active` | forwards to `list` |
| `list_durable(options)` | `list_durable` | forwards to `list` with `durable_only` |
| `list_working(options)` | `list_working` | the working records that are not durable |
| `detect_conflicts(record, now)` | `detect_conflicts` | the records that hold the same source with different content |
| `discard_working(memory_id)` | `discard_working` | drops one non-durable working record |
| `delete(memory_id)` | `delete` | drops the working entry and the durable row |
| `active_durable_records(source, now)` | `_active_durable_records` | the active durable records of a source |

### Options

```rust
pub struct AddOptions { pub durable: bool, pub promote: bool, pub now: Option<Timestamp> }
pub struct PromoteOptions { pub provenance_id: Option<String>, pub now: Option<Timestamp> }
pub struct GetOptions { pub now: Option<Timestamp>, pub include_expired: bool }
pub struct ListOptions {
    pub source: Option<String>,
    pub now: Option<Timestamp>,
    pub include_working: bool,
    pub durable_only: bool,
    pub limit: i64,
}
pub struct ListWorkingOptions { pub source: Option<String>, pub now: Option<Timestamp>, pub limit: i64 }
pub struct MemoryManagerConfig { pub max_working_records: usize, pub require_provenance_on_promotion: bool }
```

`now` is the instant the operation is evaluated at; `None` means the current
time. The Python source converts an offset value to UTC; the native
`Timestamp` compares as an absolute instant, so the same equality holds.

`PromoteTarget` selects what to promote:

```rust
pub enum PromoteTarget<'a> { Record(&'a MemoryRecord), Id(&'a str) }
```

### Results, entries and status

```rust
pub struct MemoryAddResult { /* private */ }
impl MemoryAddResult {
    pub fn status(&self) -> MemoryStatus;
    pub fn record(&self) -> &MemoryRecord;
    pub fn existing(&self) -> Option<&MemoryRecord>;
    pub fn conflicting(&self) -> Option<&MemoryRecord>;
    pub fn memory_id(&self) -> &str;
    pub fn content(&self) -> &Text;
    pub fn source(&self) -> &str;
    pub fn provenance_id(&self) -> Option<&str>;
    pub fn accepted(&self) -> bool;
    pub fn as_dict(&self) -> CanonicalValue;
}

pub struct MemoryEntry { /* private */ }
impl MemoryEntry {
    pub fn new(record: MemoryRecord) -> Self;          // durable = false
    pub fn with_durable(record: MemoryRecord, durable: bool) -> Self;
    pub fn record(&self) -> &MemoryRecord;
    pub fn durable(&self) -> bool;
}

pub enum MemoryStatus { Added, Durable, Duplicate, Conflict }
// + as_str(), is_accepted(), parse()
```

`MemoryStatus::Durable` exists so the enumeration is complete; the manager
never produces it, exactly like the Python `StrEnum`.

### Errors

```rust
pub enum MemoryError {
    Base(String),                 // MemoryError
    Validation(String),           // MemoryValidationError
    Conflict(Box<MemoryConflictError>), // MemoryConflictError
    NotFound(String),             // MemoryNotFoundError (KeyError)
    Promotion(String),            // MemoryPromotionError
    Type(String),                 // TypeError
    Value(String),                // ValueError
    Contract(ContractError),      // Pydantic ValidationError
    Repository(RepositoryError),  // accepted repository/persistence failure
}
// + python_class_name(), message(), as_conflict(), as_repository()

pub struct MemoryConflictError { /* private */ }
// + new(), record(), conflicting(), message()
```

`MemoryError::message()` renders the Python `str()` of the corresponding
exception, including the `KeyError` `repr()` rendering of a missing key and
the `repr()` of a conflicting source.

### Module-level helpers

```rust
pub fn resolve_now(now: Option<Timestamp>) -> Timestamp;   // Python _now
pub fn is_expired(record: &MemoryRecord, now: &Timestamp) -> bool; // Python _is_expired
pub fn validated_record(record: &MemoryRecord) -> Result<MemoryRecord, MemoryError>; // Python _validated_record
pub fn new_record(...) -> Result<MemoryRecord, MemoryError>;  // MemoryRecord(...) plus the module's error type
pub fn python_repr_str(value: &str) -> String;                 // CPython repr()
```

## Behavior preserved

- One content per source across the working map and the durable store. A
  different content for the same source is a conflict; the same content is a
  duplicate. The conflict check runs before the duplicate check, exactly as
  in Python.
- The working map is insertion-ordered and bounded. At capacity, expired
  entries are evicted first; if the map is still at capacity the write is
  refused with `MemoryPromotionError("working memory limit exceeded")`.
- The durable scan of one source reads at most 1,000 records, the accepted
  repository's default list limit. A source with more durable records is
  compared only against the first thousand of them.
- Expiry is inclusive: `expires_at == now` is expired. An expired durable
  record is hidden from the active listings but can still be promoted, because
  the promotion reads it with `include_expired=True`.
- Promotion refuses a provenance-less record when the manager is configured to
  require one, refuses a promotion that contradicts a durable record of the
  same source, and refuses one that contradicts another working record of the
  same source.
- Listings are ordered by `(created_at, memory_id)` and truncated by the
  limit.
- The `list` family accepts `include_working` and the Python source never
  reads it: only `durable_only` decides whether the working map is consulted.
  The native port keeps the flag and the Python behavior.
- The working map is keyed by the raw identifier the caller passed, while the
  repository normalizes identifiers. A padded identifier therefore reaches
  the durable row but not the working entry.
- `list_working` applies Python's slice semantics to its bound: it does not
  validate it, so a bound of zero returns nothing and a negative bound drops
  that many records from the end.

## Intentional differences

- The Python `TypeError` checks for a non-`MemoryRepository` repository, a
  non-integer (or `bool`) `max_working_records`, a non-integer `limit` and a
  non-record/non-string promotion target are compile-time impossibilities in
  Rust. The promotion target is the `PromoteTarget` enum instead.
- The Python `ValueError("now must include a timezone")` for a naive
  `datetime` cannot occur: the native `Timestamp` is always aware.
- The Python `MemoryValidationError("record must be a MemoryRecord")` cannot
  occur: the native `MemoryRecord` is a validated type.
- The Python `MemoryValidationError("record failed validation")` is
  unreachable for a record built through the native API, because such a
  record cannot be invalid. The reachable contract failures are reported as
  `MemoryError::Contract`, the counterpart of Pydantic's `ValidationError`.
- Pydantic renders a `ValidationError` as a multi-line report; the native
  error carries the first issue's message. The live differential compares the
  single-line form on both sides.
- The Python `list_working` bound is an arbitrary-precision `int`; the native
  bound is an `i64`.
- `repr()` of a string follows CPython except for the Unicode *Other* and
  *Separator* categories other than the ASCII space, which Rust's standard
  library does not expose. The affected characters are not valid memory
  identifiers.
- No runtime cutover, Windows runtime, speed or readiness claim is made.

## Verification

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo test --locked --release
cargo build --release --locked
```

The integration tests spawn the repository `.venv` Python (or
`KALA_MEMORY_PYTHON`) as a test oracle only, and fail rather than skip when
it is unavailable. Production code contains no Python or subprocess calls.
