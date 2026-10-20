# kala-memory — cell report (memory-r1)

## Outcome

Ported the complete Python module `src/kala_agent/kernel/memory.py` into a new
standalone Rust crate: `backend/crates/kala-memory/`.

Every public name of the Python module is ported: `MemoryError`,
`MemoryValidationError`, `MemoryConflictError`, `MemoryNotFoundError`,
`MemoryPromotionError`, `MemoryStatus`, `MemoryAddResult`, `MemoryEntry`,
`MemoryManager` (with `add`/`add_record`, `try_add`, `promote`/`promote_record`,
`get`, `list`, `list_active`, `list_durable`, `list_working`,
`detect_conflicts`, `discard_working`, `delete`), the `repository` and
`max_working_records` properties, the module-level `_now`, `_is_expired` and
`_validated_record` helpers, and the `__all__` surface.

The crate consumes the accepted `kala-repositories` (durable `MemoryRepository`
and the validated `MemoryRecord`), `kala-contracts` (`Timestamp`, `Text`,
`CanonicalJsonObject`, `ContractModel`), `kala-json` (canonical JSON values)
and `kala-persistence` (`Database`) **read-only** via path dependencies. No
accepted crate, root manifest, root lock, Python source, Python test, shared
fixture, adapter or `studio/**` file was edited by this cell. The crate is its
own workspace with its own `Cargo.lock`.

## Owned changed paths

New files only:

```text
backend/crates/kala-memory/.gitignore
backend/crates/kala-memory/Cargo.toml
backend/crates/kala-memory/Cargo.lock
backend/crates/kala-memory/README.md
backend/crates/kala-memory/src/lib.rs
backend/crates/kala-memory/src/error.rs
backend/crates/kala-memory/src/manager.rs
backend/crates/kala-memory/src/repr.rs
backend/crates/kala-memory/src/result.rs
backend/crates/kala-memory/src/status.rs
backend/crates/kala-memory/tests/oracle.py
backend/crates/kala-memory/tests/support/mod.rs
backend/crates/kala-memory/tests/memory_semantics.rs
backend/crates/kala-memory/tests/python_differential.rs
backend/crates/kala-memory/tests/security_edges.rs
.mece/waves/rust-backend-20261019/cells/memory-r1/REPORT.md
```

## Public API

Re-exported accepted types: `MemoryRecord`, `MemoryRepository`,
`CanonicalJsonObject`, `ContractModel`, `Text`, `Timestamp`, `CanonicalValue`.

### Constants

```rust
pub const DEFAULT_MAX_WORKING_RECORDS: usize = 1_000;   // Python default
pub const DURABLE_RECORD_SCAN_LIMIT: i64 = 1_000;        // repository list limit
```

### `MemoryManager<'a>`

```rust
pub fn new(repository: MemoryRepository<'a>) -> Self;
pub fn with_config(
    repository: MemoryRepository<'a>,
    config: MemoryManagerConfig,
) -> Result<Self, MemoryError>;                     // "max_working_records must be positive"
pub fn repository(&self) -> &MemoryRepository<'a>;
pub fn max_working_records(&self) -> usize;
pub fn require_provenance_on_promotion(&self) -> bool;

pub fn add(&self, record: &MemoryRecord, options: AddOptions)
    -> Result<MemoryRecord, MemoryError>;           // Python add / add_record
pub fn try_add(&self, record: &MemoryRecord, now: Option<Timestamp>)
    -> Result<MemoryAddResult, MemoryError>;
pub fn promote(&self, target: PromoteTarget<'_>, options: PromoteOptions)
    -> Result<MemoryRecord, MemoryError>;           // Python promote / promote_record
pub fn get(&self, memory_id: &str, options: GetOptions)
    -> Result<Option<MemoryRecord>, MemoryError>;
pub fn list(&self, options: ListOptions) -> Result<Vec<MemoryRecord>, MemoryError>;
pub fn list_active(&self, options: ListOptions) -> Result<Vec<MemoryRecord>, MemoryError>;
pub fn list_durable(&self, options: ListOptions) -> Result<Vec<MemoryRecord>, MemoryError>;
pub fn list_working(&self, options: ListWorkingOptions) -> Result<Vec<MemoryRecord>, MemoryError>;
pub fn detect_conflicts(&self, record: &MemoryRecord, now: Option<Timestamp>)
    -> Result<Vec<MemoryRecord>, MemoryError>;
pub fn discard_working(&self, memory_id: &str) -> bool;
pub fn delete(&self, memory_id: &str) -> Result<bool, MemoryError>;
pub fn active_durable_records(&self, source: Option<&str>, now: &Timestamp)
    -> Result<Vec<MemoryRecord>, MemoryError>;        // Python _active_durable_records
```

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
pub struct MemoryManagerConfig {
    pub max_working_records: usize,
    pub require_provenance_on_promotion: bool,
}
pub enum PromoteTarget<'a> { Record(&'a MemoryRecord), Id(&'a str) }
```

### Results, entries, status

```rust
pub struct MemoryAddResult { /* private */ }
// status(), record(), existing(), conflicting(), memory_id(), content(),
// source(), provenance_id(), accepted(), as_dict() -> CanonicalValue
// + added(), duplicate(), conflict(), new() constructors

pub struct MemoryEntry { /* private */ }
// new(record) [durable = false], with_durable(record, durable), record(), durable()

pub enum MemoryStatus { Added, Durable, Duplicate, Conflict }
// as_str(), is_accepted(), parse()
```

### Errors

```rust
pub enum MemoryError {
    Base(String),                              // MemoryError
    Validation(String),                        // MemoryValidationError
    Conflict(Box<MemoryConflictError>),        // MemoryConflictError
    NotFound(String),                          // MemoryNotFoundError (KeyError)
    Promotion(String),                         // MemoryPromotionError
    Type(String),                              // TypeError
    Value(String),                             // ValueError
    Contract(ContractError),                   // Pydantic ValidationError
    Repository(RepositoryError),               // accepted repository failure
}
// python_class_name(), message(), as_conflict(), as_repository()

pub struct MemoryConflictError { /* private */ }
// new(record, conflicting), record(), conflicting(), message()
```

`MemoryError::message()` renders the Python `str()` of the matching exception,
including `KeyError`'s `repr()` rendering of a missing key
(`MemoryNotFoundError("absent")` → `"'absent'"`) and the `repr()` of a
conflicting source
(`"memory source 'source-1' already contains different content"`).

### Module-level helpers

```rust
pub fn resolve_now(now: Option<Timestamp>) -> Timestamp;                  // Python _now
pub fn is_expired(record: &MemoryRecord, now: &Timestamp) -> bool;        // Python _is_expired
pub fn validated_record(record: &MemoryRecord) -> Result<MemoryRecord, MemoryError>; // Python _validated_record
pub fn new_record(
    memory_id: &str,
    content: &str,
    source: &str,
    provenance_id: Option<&str>,
    metadata_json: CanonicalJsonObject,
    created_at: Timestamp,
    expires_at: Option<Timestamp>,
) -> Result<MemoryRecord, MemoryError>;
pub fn python_repr_str(value: &str) -> String;                            // CPython repr()
```

## Behavior preserved (port fidelity)

- **One content per source.** The conflict check runs before the duplicate
  check, over the durable records of the source first and then over the
  working entries, exactly as in the Python `try_add`. A different content for
  the same source is `conflict`; the same content is `duplicate`.
- **Check order inside `try_add`**: durable conflict → durable duplicate →
  working conflict → working duplicate → same-identifier check → capacity
  eviction → capacity refusal → insert.
- **Same-identifier rule.** A working entry with the same `memory_id` and a
  different content is a conflict; with the same content it is a duplicate
  that returns the *stored* record as both `record` and `existing`.
- **Working map.** Insertion-ordered (`IndexMap`), bounded by
  `max_working_records`. At capacity, expired entries are evicted first; if the
  map is still at capacity the write is refused with
  `MemoryPromotionError("working memory limit exceeded")`.
- **Durable scan bound.** The source scan reads at most
  `DURABLE_RECORD_SCAN_LIMIT = 1_000` durable records — the accepted
  repository's default list limit. A source with more durable records is
  compared only against the first thousand of them. Proven both natively and
  differentially (see below).
- **Expiry is inclusive**: `expires_at == now` is expired. An expired durable
  record is hidden from the active listings and from the durable conflict
  scan, but a promotion still reads it (`include_expired=True`) and can
  rewrite it.
- **Promotion.** A record target that is already in working memory promotes
  the *stored* working record, not the offered one. A record target that is
  not in working memory is inserted first (`try_add`) and a conflict there is
  reported. The provenance override is re-validated, so an invalid
  provenance identifier fails before the store is touched. A promotion is
  refused when it contradicts a durable record of the same source, when it
  contradicts another working record of the same source, and when the manager
  requires a provenance identifier.
- **Listing.** `list` merges the working map and the durable records by
  `memory_id` (durable wins), orders by `(created_at, memory_id)` and
  truncates by the limit. `list_working` excludes durable entries.
- **`include_working` is accepted and ignored**, exactly like the Python
  source: only `durable_only` decides whether the working map is consulted.
- **`list_working` does not validate its bound** and applies Python's slice
  semantics: a bound of zero returns nothing and a negative bound drops that
  many records from the end.
- **Identifier handling.** The working map is keyed by the *raw* identifier
  the caller passed, while the repository normalizes with Python `str.strip()`
  semantics and validates the safe-identifier domain. A padded identifier
  therefore reaches the durable row but not the working entry — reproduced and
  pinned by a test.
- **Error precedence and messages.** `memory_id must be nonempty` (before the
  repository), `limit must be positive`, `max_working_records must be positive`,
  `source must be a safe identifier`, `memory_id must be a safe identifier`,
  `working memory limit exceeded`, `durable memory requires provenance`, and
  the conflict/not-found messages all match the Python text.
- **Locking.** Every public operation holds one lock across its whole
  check-then-write sequence, as the Python `RLock` does. The Python `RLock`
  is reentrant because `promote` calls the public `try_add` while holding it;
  the native port uses one non-reentrant lock and an internal
  `try_add_locked`, which is the same observable behaviour.

## Live Python differential

The differential runs the repository's own Python
(`/home/v9i/Projects/Kala-Agent/.venv/bin/python -I`, `PYTHONPATH=src`) as a
test-only oracle through `backend/crates/kala-memory/tests/oracle.py`. The
oracle executes the same operation sequences against
`kala_agent.kernel.memory` over a real in-memory SQLite database and prints
canonical JSON; the native run produces the same canonical JSON and the two
are compared **byte for byte**. Any difference in a status, a record
projection, an ordering, an expiry boundary, a limit or an error message
fails the test.

Corpus: **77 scenarios, 2,716 operations**, all matching with zero unexplained
mismatches.

| Differential test | Scenarios | Operations | Covers |
|---|---|---|---|
| `recording_querying_and_promoting_match_python` | 1 | 13 | add/try_add/promote, durable duplicate, durable conflict, delete, list |
| `expiry_boundaries_match_python` | 1 | 9 | expiry inclusive boundary, `include_expired`, expired working/durable records |
| `listing_sources_limits_and_ordering_match_python` | 1 | 16 | source filters, limits, `(created_at, memory_id)` ordering, ignored `include_working`, working-list slice bounds |
| `working_memory_eviction_and_capacity_match_python` | 3 | 8 | eviction of expired entries at capacity, capacity refusal, invalid capacity |
| `promotion_rules_match_python` | 2 | 11 | promotion by id/record, provenance override, invalid provenance, missing id, stored-record-wins, provenance requirement |
| `conflict_detection_and_discarding_match_python` | 1 | 8 | `detect_conflicts` ordering/filtering, `discard_working` |
| `invalid_arguments_match_python` | 1 | 8 | empty/blank/hostile identifiers, hostile source, non-positive limits, hostile delete |
| `offsets_and_instant_equality_match_python` | 1 | 4 | offset-equivalent instants |
| `randomized_corpus_matches_python` | 64 | 632 | seeded pseudo-random operation sequences (11 operation kinds, 6 ids, 3 sources, 4 contents, 4 instants, random capacities and provenance requirements) |
| `durable_scan_limit_matches_python` | 2 | 2,007 | the 1,000-record durable window, including the discriminating case (1,000 records with one content, the 1,001st with another) |

Extra assurance runs performed during development (not part of the committed
suite): the randomized corpus was re-run with 600 scenarios under three
additional seeds (`0xA5A5_0000_0000_0007`, `0x1111_2222_3333_4444`,
`0xDEAD_BEEF_CAFE_0001`, `0x0BAD_C0DE_1234_5678`) — 1,800 further scenarios,
all byte-identical to the Python oracle.

The differential is also proven to be able to fail: temporarily changing the
expiry comparison from `<=` to `<` in the native code made
`expiry_boundaries_match_python`, `working_memory_eviction_and_capacity_match_python`
and `randomized_corpus_matches_python` fail. The change was reverted.

## Security and edge-case evidence

`backend/crates/kala-memory/tests/security_edges.rs` (18 tests):

- **Identifier handling at every boundary.** `get`, `delete`, `list(source=…)`
  and `promote(id)` refuse `../memory`, `m-1/../m-2`, `m 1`, `m-1!` and
  `-m-1` with the Python messages; an empty or whitespace-only identifier is
  refused earlier with `memory_id must be nonempty`.
- **Raw-versus-normalized identifiers.** `delete(" m-1 ")` removes the durable
  row and leaves the working entry, exactly as in Python.
- **Content bounds.** Whitespace-only content is refused; 16,384 code points
  are accepted and 16,385 are refused; 16,000 astral code points (64,000
  UTF-8 bytes) are accepted, proving the limit counts code points.
- **Metadata.** Canonical JSON object only; the stored metadata is the
  canonical (sorted, ASCII) text.
- **No stack overflow on attacker-sized input.** 20,000 levels of nested
  metadata are accepted; the canonical JSON parser and serializer are
  iterative, so nothing recurses on the depth.
- **Bounds.** The working map refuses the write past its capacity; the durable
  scan is proven to stop at 1,000 records; `list` refuses a non-positive limit
  (including `i64::MIN`); `list_working` uses Python slice semantics.
- **Expiry.** An expired durable record is hidden but promotable; an expired
  record does not block a new content for the same source.
- **Promotion guards.** An invalid provenance override is refused before the
  durable store is touched (the durable listing stays empty).

`backend/crates/kala-memory/tests/memory_semantics.rs` (53 tests) is a
one-to-one port of the memory tests in
`tests/unit/test_kernel_memory_planning.py` and of the memory test in
`tests/unit/test_kernel.py`, including the concurrent-add test (10 threads,
one manager, no lost updates).

## Verification

All commands run in `backend/crates/kala-memory`.

| Command | Result |
|---|---|
| `cargo fmt --all --check` | clean |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | clean |
| `cargo test --locked` (debug) | **85 passed, 0 failed, 0 ignored** |
| `cargo test --locked --release` | **85 passed, 0 failed, 0 ignored** |
| `cargo build --release --locked` | clean |
| `cargo check --locked --target x86_64-pc-windows-gnu` | **blocked by the host** (see below) |

Test distribution: `src/repr.rs` unit tests 3, `memory_semantics` 53,
`python_differential` 10, `security_edges` 18, doctest 1 — **85 total**.

Python reference regressions (read-only, to confirm nothing in the reference
implementation drifted):

| Command | Result |
|---|---|
| `pytest tests/unit/test_kernel_memory_planning.py` | 96 passed |
| `pytest tests/unit/test_kernel.py tests/unit/test_persistence.py` | 28 passed |

### Windows cross-check

`cargo check --locked --target x86_64-pc-windows-gnu` fails in the **accepted**
`libsqlite3-sys` custom build command:

```text
error: failed to run custom build command for `libsqlite3-sys v0.38.2`
  process didn't exit successfully: .../build/libsqlite3-sys-*/build-script-build
warning: libsqlite3-sys@0.38.2: Compiler family detection failed due to error:
  ToolNotFound: failed to find tool "x86_64-w64-mingw32-gcc": No such file or directory
```

The `x86_64-pc-windows-gnu` Rust target **is** installed, but the MinGW C
compiler is not present on this host, so the bundled SQLite of the accepted
persistence layer cannot be compiled for Windows. The identical failure occurs
for the accepted `kala-repositories` crate, which proves it is inherited from
the accepted dependency and not introduced by this cell. The only error
mentions the C build script; nothing in `kala-memory` itself is implicated. No
Windows compile or runtime claim is made.

## Intentional differences (complete list)

1. **Type checks are compile-time.** The Python `TypeError("repository must be
   a MemoryRepository")`, `TypeError("max_working_records must be an
   integer")`, `TypeError("limit must be an integer")` and `TypeError
   ("record_or_id must be a MemoryRecord or memory_id")` cannot occur: the
   repository is a typed reference, the capacities and limits are typed
   integers, and the promotion target is the `PromoteTarget` enum. The
   dynamic `ValueError` counterparts (`max_working_records must be positive`,
   `limit must be positive`) still occur and keep their exact text.
2. **Naive timestamps are impossible.** The Python
   `ValueError("now must include a timezone")` cannot occur: the native
   `Timestamp` is always aware. An offset value is not rewritten to UTC, but
   the native type compares as an absolute instant, so every comparison the
   Python performs after `astimezone(UTC)` is identical.
3. **`MemoryValidationError` is mostly unreachable.** The native
   `MemoryRecord` is a validated type, so `"record must be a MemoryRecord"`
   and `"record failed validation"` cannot be produced for a record built
   through the typed API. The reachable contract failures are reported as
   `MemoryError::Contract`, the counterpart of Pydantic's `ValidationError`.
4. **Pydantic error rendering.** Pydantic renders a `ValidationError` as a
   multi-line report with per-field details; the native error carries the
   first issue's message. The live differential normalizes the oracle to the
   same single message (`"Value error, "` prefix removed) so the two sides are
   compared byte for byte; the *class* and the *first message* agree.
5. **Exception hierarchy.** Python's five-class hierarchy becomes one
   `MemoryError` enum plus `python_class_name()` / `message()`, following the
   pattern the accepted `kala-kernel` crate already uses for `PolicyError` and
   `WorkflowError`. The `MemoryConflictError` value is boxed so the error
   type stays small.
6. **Arbitrary-precision bounds.** The Python `list_working` bound is an
   arbitrary-precision `int`; the native bound is an `i64`.
7. **`repr()` of a string.** CPython escapes every non-printable character;
   Rust's standard library exposes the *Control* and *White_Space* properties
   but not the full general-category tables, so the unassigned (`Cn`), format
   (`Cf`), surrogate (`Cs`) and private-use (`Co`) categories are rendered
   verbatim instead of escaped. The affected characters are not valid memory
   identifiers, so the difference is unobservable for this module's messages.
8. **Two Python tests need private-state injection.** The Python tests
   `test_promote_reports_conflict_against_durable` and
   `test_promote_reports_conflict_against_other_working_records` write
   directly into `manager._working`. The native working map is private, so
   the same end state is built through the public API (a working record
   entered before the durable record exists, and an expired working record
   that no longer conflicts). The asserted outcome — the conflicting
   identifier — is identical.
9. **Lock type.** The Python `RLock` is reentrant; the native lock is not,
   because the one nested call (`promote` → `try_add`) is an internal
   `try_add_locked` that assumes the lock is held. The observable behaviour
   is the same, and the concurrent-add test passes.
10. **No second database abstraction.** The manager borrows the accepted
    `MemoryRepository` over the accepted `Database`; no schema, pool, lock
    manager or repository was duplicated.

## Remaining gaps / not claimed

- No runtime cutover, HTTP/CLI/kernel integration, or Python deletion was
  performed. This cell is the memory module only.
- No Windows runtime claim (host toolchain missing, see above).
- No performance benchmark or speedup claim was made.
- The `now`-defaults-to-now path cannot be compared against Python
  deterministically; it is covered by a native test
  (`now_defaults_to_current_utc`) and by the fact that every differential
  operation passes an explicit instant.
- `MemoryError::Validation` is currently unreachable through the public API
  (see difference 3). It is kept so the port of the Python hierarchy is
  complete.
- No commit, push, publish, deployment or delegation was performed, and no
  credential was read.

## Repair rounds

No conductor-requested repair round was needed. The development-time failures
(two NUL-byte/formatting slips in the test sources and three test expectations
that had to be corrected against the real Python semantics — the empty
identifier, the padded-identifier delete and the durable-scan-window shape)
were fixed while building the tests; they were not separate repair rounds.
