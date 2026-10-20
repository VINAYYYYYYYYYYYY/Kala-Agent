# kala-repositories

Native typed repositories over the accepted Kala Agent persistence schema and
contracts. This crate is the Rust counterpart of
`kala_agent.persistence.repositories` and owns **no schema, migration,
connection pool, lock manager or second database abstraction**. Every
repository borrows the accepted `kala-persistence::Database`, so all callers
share one serialized SQLite writer and the frozen schema-v2 tables, triggers,
foreign keys and `BEGIN IMMEDIATE`/`SAVEPOINT` semantics.

It is a standalone workspace with its own `Cargo.lock`; it does not modify the
root backend workspace or any accepted crate.

## Accepted dependencies (read-only)

- `kala-persistence` — `Database`, frozen schema v2, transaction guard,
  timestamp formatting, `PersistenceError`.
- `kala-contracts` — validated immutable `DesignSpec`, `DesignRevision`,
  `ApprovalRequest`, `ToolCall`, `ToolResult`, `Timestamp`, `Text`,
  `CanonicalJsonObject`, `Identifier`, `ContractModel`.
- `kala-json` — canonical JSON value/limits used by metadata and
  re-serialization checks.

`kala-domains` and `kala-process` are deliberately not dependencies.

## Public API

```rust
use kala_repositories::{
    ApprovalRequestRepository, Database, DesignRevisionRepository, DesignSpecRepository,
    MemoryGetOptions, MemoryListOptions, MemoryRecord, MemoryRepository,
    ToolCallRepository, ToolResultRepository,
};

let database = Database::open("state/kala.sqlite3")?;
let specs = DesignSpecRepository::new(&database);
let revisions = DesignRevisionRepository::new(&database);
let approvals = ApprovalRequestRepository::new(&database);
let calls = ToolCallRepository::new(&database);
let results = ToolResultRepository::new(&database);
let memory = MemoryRepository::new(&database);
```

Repositories are `Copy` borrows of `&Database`, so constructing several over
the same database is the intended sharing model.

### DesignSpecRepository

| Method | Python counterpart | Behavior |
|---|---|---|
| `save(&DesignSpec)` | `save` | `INSERT ... ON CONFLICT(id) DO UPDATE`, transactional |
| `get(&str)` | `get` | normalized identifier, one row |
| `list()` | `list()` | `ORDER BY id LIMIT 1000` |
| `list_with_limit(i64)` | `list(limit=...)` | positive limit, else `InvalidLimit` |

### DesignRevisionRepository

| Method | Python counterpart | Behavior |
|---|---|---|
| `commit_revision(&DesignRevision, Option<&str>)` | `commit_revision` | required expected-head CAS, returns the revision |
| `get(&str)` | `get` | normalized revision identifier |
| `list_for_design(&str)` | `list_for_design` | ascending `sequence`, default limit 1000 |
| `list_for_design_with_limit(&str, i64)` | `list_for_design(limit=...)` | positive limit |
| `latest_for_design(&str)` | `latest_for_design` | descending `sequence LIMIT 1` |

There is intentionally **no** `save`, `replace`, `upsert` or unconditional
insert. `commit_revision` is the only write path. In one immediate transaction
it reads the current head, compares it with the required expected head,
validates `sequence` (`0` for an unborn design, otherwise head + 1), inserts
the spec with `ON CONFLICT(id) DO NOTHING`, and inserts the revision. Any
failure — including a constraint failure or a failed `COMMIT` — rolls back
every row. The loser receives `RepositoryError::HeadMoved` with all Python
fields:

```text
message, design_id, expected_head_revision_id, actual_head_revision_id,
expected_sequence (BigInt), actual_sequence (BigInt)
```

Sequences are arbitrary-precision in the error so a huge Python-style
`sequence` is reported exactly. A *matching* head at SQLite's `i64::MAX`
produces the documented `SequenceOutOfRange` instead of binding an
unrepresentable `INTEGER`.

### ApprovalRequestRepository

| Method | Python counterpart | Behavior |
|---|---|---|
| `save(&ApprovalRequest)` | `save` | insert or update while pending; decided rows only byte-identical no-ops |
| `get(&str)` | `get` | normalized request identifier |
| `list_for_workflow(&str)` | `list_for_workflow` | `created_at ASC, request_id ASC`, default limit 1000 |
| `list_for_workflow_with_limit(&str, i64)` | `list_for_workflow(limit=...)` | positive limit |

The conditional UPSERT is the actual SQL:

```sql
... ON CONFLICT(request_id) DO UPDATE SET ...
WHERE approval_requests.status = 'pending'
   OR approval_requests.model_json = excluded.model_json
```

A zero-row update raises `ApprovalAlreadyDecidedError`. Because the condition
tests `model_json`, a byte-identical write still executes the UPDATE and repairs
tampered side columns (status/created_at/workflow_id) without reopening the
decision. A changed model, including reopening as pending, is refused.

### ToolCallRepository

| Method | Python counterpart | Behavior |
|---|---|---|
| `save(&ToolCall)` | `save` | insert or replace, transactional |
| `get(&str)` | `get` | normalized call identifier |
| `list_for_tool(&str)` | `list_for_tool` | `issued_at ASC, call_id ASC`, default limit 1000 |
| `list_for_tool_with_limit(&str, i64)` | `list_for_tool(limit=...)` | positive limit |

### ToolResultRepository

| Method | Python counterpart | Behavior |
|---|---|---|
| `save(&ToolResult)` | `save` | checks call existence and `tool_id` identity then upserts, one transaction |
| `get(&str)` | `get` | normalized call identifier |

Missing calls and mismatched tool identities raise
`RepositoryError::ToolResultMissingCall` / `ToolResultToolMismatch`; existing
rows are untouched and the schema foreign key remains the final backstop.

### MemoryRepository and MemoryRecord

| Method | Python counterpart |
|---|---|
| `put(&MemoryRecord)` | `put` |
| `get(&str)` | `get` at the current instant |
| `get_with_options(&str, &MemoryGetOptions)` | `get(now=..., include_expired=...)` |
| `list()` | `list()` at the current instant, default limit 1000 |
| `list_with_options(&MemoryListOptions)` | `list(source=..., now=..., include_expired=..., limit=...)` |
| `delete(&str) -> bool` | `delete` |

`MemoryGetOptions` and `MemoryListOptions` default to `now = None` (current
instant), `include_expired = false`, and `limit = 1000`. Builders:
`with_now`, `with_include_expired`, `with_source`, `with_limit`. Expiry is the
Python boundary exactly: a row is active only when
`expires_at IS NULL OR expires_at > now`, so equality is expired.

`MemoryRecord` is a native validated immutable value implementing
`ContractModel` (the same Pydantic-compatible model JSON parser/serializer as
the accepted contracts):

- `memory_id`, `source`, optional `provenance_id`: normalized `Identifier`;
- `content`: Pydantic `NonEmptyText` semantics — Unicode White_Space trim,
  1..=16,384 code points, no lone surrogates;
- `metadata_json`: canonical JSON object in the unbounded persistence domain
  (no 48-depth or 1 MiB tool caps; CPython's 4,300-digit integer limit is
  retained);
- `created_at`, optional `expires_at`: aware timestamps; `expires_at` must be
  strictly later as an absolute instant;
- extra model JSON fields are forbidden and outer model JSON duplicate keys are
  last-wins (Pydantic), while duplicate keys inside `metadata_json` text are
  rejected (canonical object).

`MemoryRecord::new(...)` constructs a value from typed parts;
`MemoryRecord::from_json_str` / `from_json_bytes` (via `ContractModel`) apply
the exact model JSON validation. `to_wire_json` emits the same field order and
bytes as `model_dump_json()` and is what the repository persists.

## Errors

`RepositoryError` distinguishes:

- `InvalidIdentifier { field }` — Python `ValueError("<field> must be a safe identifier")`;
- `InvalidLimit` — Python `ValueError("limit must be positive")`;
- `Validation(ContractError)` — a typed model could not be serialized;
- `StoredModel { model, source }` — tampered/invalid stored `model_json`;
- `ToolResultMissingCall` / `ToolResultToolMismatch`;
- `SequenceOutOfRange` — matching head at SQLite's integer ceiling;
- `TimestampOutOfRange`;
- `HeadMoved(Box<HeadMovedError>)`;
- `ApprovalAlreadyDecided(ApprovalAlreadyDecidedError)`;
- `Persistence(PersistenceError)` — accepted SQLite/persistence failures.

Helpers: `as_head_moved`, `as_approval_already_decided`, `as_persistence`,
`as_sqlite`. A raw SQLite constraint/trigger rejection remains inspectable via
`as_sqlite()`.

The accepted `Database::transaction` signature only carries
`PersistenceError`. The repositories record their typed error beside the
closure and abort the accepted transaction with a private sentinel so the
accepted guard performs the normal rollback (including savepoint rollback and
failed-COMMIT cleanup); the recorded error is returned to the caller. The
sentinel is never observable through the public API.

## Compatibility notes and limits

- SQL text, order-by clauses, tie breakers, default limits and upsert races
  mirror the Python source. Identifiers are `str.strip()`-normalized at every
  dynamic lookup boundary.
- `model_json` decoded on read uses the accepted contract validation. Reads
  never rewrite or canonicalize the stored bytes.
- Writers emit Pydantic-style compact JSON. Live cross-reader tests compare raw
  `model_json` bytes and UTC microsecond `created_at`/`expires_at` columns in
  both directions.
- Python's `limit: int` is typed as `i64`; non-integer and `bool` values are
  compile-time impossibilities, and values outside `i64` are not representable.
  Non-positive values fail identically at runtime.
- Stored model validation failure is a typed `StoredModel` error rather than
  Pydantic's `ValidationError`; it identifies the model name and wraps the
  accepted `ContractError`.
- A revision `sequence` that matches the head but exceeds SQLite's signed
  64-bit `INTEGER` returns `SequenceOutOfRange`; Python's `sqlite3` raises
  `OverflowError` at the insert.
- The Python source's `TypeError("spec must be a DesignSpec")`-style checks are
  impossible with typed references.
- No Windows runtime claim: `kala-persistence` uses bundled SQLite and a
  Windows cross-check needs an MSVC `lib.exe`; that is host-gated and reported
  in this cell's `REPORT.md`.

## Verification

```bash
cargo fmt --all --check
cargo test --locked
cargo test --locked --release
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo build --release --locked
```

The cross-reader tests spawn the repository `.venv` Python (or
`KALA_REPOSITORIES_PYTHON` / `KALA_PYTHON`) as a test oracle only. They fail
rather than silently skip when the oracle is unavailable. Production code has
no Python or subprocess calls.
