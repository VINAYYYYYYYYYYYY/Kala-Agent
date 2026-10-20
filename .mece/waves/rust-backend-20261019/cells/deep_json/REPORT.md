# Deep JSON compatibility cell report

**Status: implementation complete for this cell's slice plus the conductor's
narrow identifier-normalization exception; no commit, push, deployment, model
change or delegation was made, and no performance claim is offered.** The
former native 1,024-level recursive JSON ceiling is gone. Persisted
`CanonicalJsonObject` values that the live Python product accepts at the tested
depths now parse, validate, encode, compare, clone, print and drop through
stack-safe native code on ordinary 2 MiB thread stacks.

## 1. Ownership and files

Owned and changed or created:

- `backend/crates/kala-json/src/parse.rs`
- `backend/crates/kala-json/src/dump.rs`
- `backend/crates/kala-json/src/value.rs`
- `backend/crates/kala-json/src/limits.rs` (documentation only)
- `backend/crates/kala-json/tests/deep_json.rs` (new)
- `backend/crates/kala-persistence/src/canonical.rs`
- `backend/crates/kala-persistence/src/journal.rs` (conductor-authorized
  identifier-normalization predicate only)
- `backend/crates/kala-persistence/tests/canonical_domain.rs`
- `backend/crates/kala-persistence/tests/deep_json_compat.rs` (new)
- `backend/crates/kala-persistence/tests/deep_json_persistence.rs` (new)
- `backend/crates/kala-persistence/tests/deep_json_cross_reader.rs` (new)
- `backend/crates/kala-persistence/tests/identifier_whitespace_compat.rs`
  (new)
- `backend/crates/kala-persistence/tests/deep_json_oracle.py` (new, test-only)
- `backend/README.md` (narrow status update)
- this `REPORT.md` and `SESSION.md`

Not changed:

- `backend/Cargo.toml`, `backend/Cargo.lock` (no dependency or workspace
  change was needed; `cargo metadata --no-deps` still reports exactly
  `kala-json` and `kala-persistence`)
- `backend/fixtures/**`, `backend/compat/**` (SHA-256 below)
- other `kala-persistence` source/tests, other crate manifests/sources/locks
- `src/**`, `tests/**`, `studio/**`, `adapters/**`, packaging, docs and
  user-owned files
- active `kala-studio`, `kala-step`, `kala-geometry` and `kala-budget` files

Fixture hashes after all runs:

```text
9d8b6556c99006ce287a52c65e2590ef5d2b485ec73bdd4079fb648191025832  fixtures/corpus/v1/schema_v2.sqlite3
4cc4dc39e3900ce0b22aa58230ed05e66290351b9be0005f0e0d51ecd93420c7  fixtures/corpus/v1/event_chain.json
d8a2fefcd5d70f05556938896bada27affa2a1a8288d3d1a788d17b1fde4ba07  fixtures/corpus/v1/canonical_json.json
```

The first two match the accepted persistence report and the tree has no
fixture/compat file with a modification time in this cell's window.

## 2. What changed

### 2.1 Iterative parser (`kala-json`)

`parse_strict` now uses an explicit `Vec<Frame>` state machine:

- `Frame::Array(Vec<CanonicalValue>)` and
  `Frame::Object { entries, key }` hold opened containers.
- A completed value is attached to the top frame; closing a container pops it
  and attaches the aggregate to its parent. No recursive `parse_value`,
  `parse_object` or `parse_array` calls remain.
- String, number, escape, surrogate, duplicate-key, non-finite, UTF-8, BOM,
  trailing-data, byte-cap and integer-digit rules are unchanged.
- `PARSER_RECURSION_LIMIT = 1024` was removed. The tool-protocol depth domain
  is still exactly 48 because the iterative validator applies
  `limits.max_json_depth` to every value with root depth zero.
- Tool input remains bounded: the heap parser stack is proportional to the
  actual syntactic nesting and the input is already capped at 1 MiB. A
  one-MiB unmatched-bracket document was exercised and returns `NotJson`
  without crashing; a document one byte over the cap returns `InputTooLarge`.

`parse_strict` still parses fully and then validates. This preserves the
existing order for errors that occur during parsing (syntax, duplicate keys,
non-finite tokens) and only the already-documented "several independent
violations" precedence can differ. For well-formed tool input deeper than the
old 1,024 recursion cap, the result is now `DepthExceeded` instead of
`NotJson`; both reject the document and the accepted set is unchanged.

### 2.2 Iterative validation and encoding (`kala-json`)

`validate` and `canonical_dumps` no longer recurse:

- Validation uses a `Check` work stack. It applies depth, integer-digit,
  finite-float, string, array-count and key-length checks in the same
  pre-order as before, with object key checks before their values.
- Encoding uses an `Encode` work stack of value/key/literal tasks. It emits
  the same CPython-compatible bytes, sorted keys and separators as before.
- Output is built once in a `String`; the byte cap is still applied to the
  final ASCII bytes. There is no reconstruct-and-reencode or quadratic string
  concatenation.
- `usize::MAX` in `max_json_depth` is the documented no-bound sentinel.

### 2.3 Stack-safe value lifecycle (`kala-json`)

`CanonicalValue` keeps its exact public enum shape and variant types, but
`Debug`, `Clone`, `PartialEq` and `Drop` are now manual iterative
implementations:

- **Drop** detaches a container's children with `std::mem::take` into a heap
  work list. An emptied container's own `Drop` returns immediately, so no
  recursive deallocation remains. There is no `mem::forget`, no unsafe code,
  and no leak. `#![forbid(unsafe_code)]` remains in force.
- **Clone** is a post-order traversal. Child results are collected on a
  `results` stack and a container consumes exactly its children; each node is
  cloned once. No recursive enum clone remains.
- **PartialEq** compares `(left, right)` pairs on an explicit stack, including
  sorted object key equality before child values.
- **Debug** reproduces the derived formatting (`Null`, `Bool(..)`,
  `Array([..])`, `Object({JsonText(".."): ..})`, etc.) with an explicit task
  stack, verified by a shallow exact-shape test and by deep formatting.
- Because `CanonicalValue` now implements `Drop`, field move-out patterns are
  forbidden by the compiler. No own or dependent use moved fields out of the
  enum; all existing constructors, accessors, `match`es and consuming uses
  compile unchanged. No ergonomic helper was required.

The public API signatures, `CanonicalObject` alias, `Limits` fields, error
enum, digest/chain APIs and `JsonText` behavior are unchanged.

### 2.4 Persistence domain

`persistence_limits()` now sets:

```rust
max_json_bytes: usize::MAX,
max_string_length: usize::MAX,
max_collection_items: usize::MAX,
max_key_length: usize::MAX,
max_json_depth: MAX_PERSISTENCE_JSON_DEPTH, // usize::MAX
max_integer_digits: 4_300,
```

`MAX_PERSISTENCE_JSON_DEPTH` remains exported for source compatibility but is
now the `usize::MAX` no-bound sentinel, documented as such. Persistence no
longer inherits the old arbitrary 1,024 syntactic ceiling. The 4,300-digit
CPython integer limit is preserved because both Python `json.loads` and
`json.dumps` enforce it.

### 2.5 Identifier normalization (authorized narrow expansion)

The conductor authorized `journal.rs` identifier normalization only. Live
Python `_normalize_identifier` uses `str.strip()`, whose whitespace set
includes `U+001C..=U+001F` in addition to the Unicode `White_Space` set used by
Rust `str::trim`. A new `python_strip`/`is_python_whitespace` pair now covers
exactly CPython's 29 `str.isspace()` code points. It is used only by:

- append identifier validation (`validate_identifier`),
- the typed event view after raw-byte chain verification
  (`normalized_identifier`), and
- the stored-row identifier-shape check in `validate_event_row`.

Chain hashing still uses the **raw stored identifier bytes**; the accepted P1
raw-chain repair is untouched. Timestamps, transactions, schema, triggers and
other journal logic were not changed. A correctly rehashed raw row padded with
`U+001C` is accepted and exposed with normalized identifiers in both
Rust -> Python and Python -> Rust directions; an append extends its raw head.

## 3. Deep compatibility evidence

### 3.1 Canonical bytes and digests against live Python

`deep_json_compat.rs` compares `contracts.models._canonical_json_object` with
native parse + canonical dump for:

| Case | Depth | Content |
|---|---:|---|
| mixed-1500 | 1,500 | alternating arrays/objects, Unicode, bigint, `-0.0`, null |
| arrays-5000 | 5,000 | nested arrays |
| mixed-5000 | 5,000 | alternating arrays/objects and mixed leaf |
| objects-1500 | 1,500 | nested objects with distinct keys, testing sorted-key output |
| mixed-10000 | 10,000 | alternating arrays/objects and mixed leaf |
| mixed-20000 | 20,000 | alternating arrays/objects and mixed leaf |

Every Python-accepted case matches byte-for-byte, including output length and
SHA-256. Each result is reparsed, re-encoded and required to be stable. The
same sources are refused by the tool domain with `DepthExceeded` above 48.

### 3.2 Stack-safety of every lifecycle operation

`kala-json/tests/deep_json.rs` runs on explicit 2 MiB threads:

- mixed values at 1,500 / 5,000 / 10,000 / 20,000 / 50,000: parse, dump,
  reparse, clone, equality, `Debug`, explicit drop;
- typed deep values at 5,000 / 20,000 / 50,000: clone, equality, `Debug`,
  drop, canonical parse/dump;
- malformed deep input (missing closers, trailing data, unterminated string,
  bad number, missing comma, duplicate key after a completed deep value) at
  20,000 levels: clean error and cleanup;
- tool depth 48 accepted; 49 / 1,500 / 20,000 refused with `DepthExceeded`;
  one-MiB unmatched `[` refused with `NotJson`; one byte over the cap refused
  with `InputTooLarge`;
- 200,000-element array and 20,000-key object: parse, dump, clone, equality
  and drop without quadratic rebuilding (test guard 30 s; the whole
  seven-test debug binary ran in 0.29 s).

### 3.3 Persistence lifecycle

`deep_json_persistence.rs` exercises a fresh database:

- `append_value` with a typed 20,000-level object; `append` with a 15,000-level
  array; `append`/`append_value` bytes and digests equal canonical output;
- `get`, `read`, `read_filtered` (event type/aggregate/workflow/run/after/
  limit), `verify_chain` and `Event::payload()` on the deep payload;
- deep snapshot `save_value`/`save`/`get`/`latest`/`restore`/`restore_latest`;
- close/reopen with full chain re-verification and deep bytes intact.

It also appends a 20,000-level event and 5,000-level snapshot to a temporary
copy of `schema_v2.sqlite3`, verifies the extended six-row chain, reopens and
restores; the source fixture is untouched.

### 3.4 Real Python <-> Rust cross-readers

`deep_json_cross_reader.rs`:

- Rust creates a database with a 5,000-level event and 2,500-level snapshot;
  Python opens, fully verifies, and matches canonical bytes, payload digest,
  snapshot digest and head.
- Python appends a 10,000-level event and snapshot to that database; Rust
  reopens, verifies the new head and bytes, then appends a 20,000-level event
  and 10,000-level snapshot; Python verifies the final three-row chain.
- Python creates a database with a 10,000-level event and snapshot; Rust opens,
  verifies bytes/digests/head, parses the deep payload, appends a
  20,000-level event and 10,000-level snapshot; Python verifies the final
  two-row chain.
- A temporary fixture copy is extended by Rust with a 20,000-level event and
  5,000-level snapshot; Python verifies the six-row chain and final bytes.

## 4. Exact deep range and runtime-limit difference

Measured on this host with CPython 3.14.8 in an ordinary oracle child process
(the `.venv/bin/python` interpreter; default process stack), comparing live
`_canonical_json_object`:

- Python accepted 1,500 / 5,000 / 10,000 / 20,000 mixed documents with exact
  native byte parity.
- Python accepted pure nested-array documents at 25,000 and 40,000.
- Python failed at 60,000 and 100,000 with the wrapped recursion error
  `ValueError: value must be a valid JSON object`.
- Native accepted and round-tripped every probed depth, including 60,000 and
  100,000; native mixed lifecycle values were also exercised at 50,000.
- The persistence report's earlier 52,120 nested-array observation is
  consistent with this boundary (between 40,000 and 60,000). The exact CPython
  threshold is process- and stack-dependent, not a product contract.

Thus the tested Python-interoperable stored-data range is safely through
20,000-level mixed objects and 40,000-level pure nesting; native well-formed
documents beyond that range are accepted, and the Python runtime limit is the
only reason a cross-reader Python check cannot confirm them. The test prints
this measured boundary; it does not hide a Python failure as native success.
The tool domain remains exactly 48; the native persistence domain has no fixed
depth number.

## 5. Checks run

Host: Linux x86_64, `cargo/rustc 1.99.1`, oracle
`backend/.venv/bin/python` (CPython 3.14.8). From `backend/`:

| Command | Result |
|---|---|
| `cargo fmt --all --check` | exit 0 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| `cargo test --workspace --locked` | **98 passed**, 0 failed/ignored |
| `cargo test --workspace --release --locked` | **98 passed**, 0 failed/ignored |
| `cargo build --release --workspace --locked` | exit 0 |
| `cargo metadata --no-deps --locked` | exactly `kala-json`, `kala-persistence` |

Workspace breakdown (same in debug and release):

- `kala-json`: 20 unit + 3 corpus + 7 deep + 3 Python differential = 33
- `kala-persistence`: 9 unit + 5 canonical domain + 4 deep compat + 3 deep
  cross-reader + 2 deep persistence + 3 identifier whitespace + 4 journal
  fixture + 13 journal security + 3 Python cross-reader + 16 schema migration
  + 3 timestamp = 65

The baseline was 76 root tests; this cell adds 22 (3 parser edge unit tests, 7
deep lifecycle, 4 deep compatibility, 3 deep cross-reader, 2 deep persistence,
3 identifier whitespace).

Dependent checks (source not edited):

| Crate | Command | Result |
|---|---|---|
| `kala-budget` | `cargo test --release --locked` | **65 passed**, 0 failed |
| `kala-studio` | `cargo test --locked` | **65 passed**, 0 failed |

Release is compilation/testing evidence only; no benchmark or speedup is
claimed. No throughput, RSS or latency numbers are reported.

## 6. Algorithmic observations (validation only)

- Parser: O(input bytes), heap frame stack proportional to open nesting.
- Validator/encoder: O(nodes), single traversal; encoder writes directly into
  one output `String`.
- Clone: post-order O(nodes); each node cloned once, child results moved into
  parents with `split_off`.
- Equality/debug: O(nodes), explicit stacks; debug emits the same text as the
  derived implementation.
- Drop: O(nodes), one heap work list; no recursive deallocation and no value
  leak.
- Wide containers (200,000 array items / 20,000 object keys) complete well
  inside the 30-second guard; no quadratic rebuilding is observed.

## 7. Gaps and blockers

1. **CPython runtime boundary.** Python's recursive JSON decoder/encoder fails
   at a process/stack-dependent depth (between 40,000 and 60,000 here). Native
   handles well-formed documents beyond that, so the accepted sets differ by
   design for documents larger than Python's incidental runtime limit. This is
   explicitly reported, not claimed as identical parity.
2. **Tool error precedence.** Well-formed tool input deeper than the former
   1,024 parse-recursion cap now reports `DepthExceeded` instead of the old
   `NotJson`; malformed deep input still fails closed (usually `NotJson`).
   Accepted/rejected sets and the 48-level limit are unchanged.
3. **Heap memory for adversarial tool input.** The parser stack is proportional
   to syntactic nesting and bounded by the 1 MiB tool byte cap; a one-MiB
   unmatched-bracket probe was clean but does allocate a large temporary frame
   vector. Persistence intentionally permits larger documents and uses memory
   proportional to the source contract.
4. **Windows.** No MSVC/Windows build or COM execution was performed here.
5. **Scope.** This is not full backend completion: services/core/runtime/CAS/
   typed repositories/Windows CAD/packaging and controlled release benchmarks
   remain for later cells.
6. **Python oracle.** Tests fail rather than skip when Python is unavailable,
   as before. The deep oracle is test-only and not a production fallback.
7. **No commit/push/deploy** was performed and no user-owned file was changed.

The persistence slice now reads and writes the deep data that the Python
product already accepts in the tested range. Reviewer acceptance of this cell
is still required before any integration/cutover claim.
