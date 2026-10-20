# native-journal — the Studio session journal and artifact store in Rust

Cell: `native-journal` (wave `cutover-20261020`). Spec: the cell task, ADR-0009 §5
and acceptance criterion 5, and the Rust `CONVENTIONS.md`.

Status: **done at library level.** The native journal writes the *same bytes* as the
Python journal, opens and extends a Python-written journal, and detects the same
tampering. No commit, no push, no delegation, no credential reads, no sudo.

**The user's real data was never touched.** Every shell ran with
`HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME` pointed at
fresh `mktemp -d` directories, and every test writes only into a `tempfile::TempDir`.
The path `~/.local/share/kala-agent/**` was never read, written or even named in a
command.

---

## 1. Changed paths

Only the new crate was created. Nothing else in the repository was edited.

| Path | Lines | Note |
|---|---|---|
| `backend/crates/kala-studio-journal/Cargo.toml` | 35 | new — standalone workspace, own lockfile |
| `backend/crates/kala-studio-journal/Cargo.lock` | — | new — the crate's own lockfile |
| `backend/crates/kala-studio-journal/README.md` | 71 | new — the crate's own README |
| `backend/crates/kala-studio-journal/src/lib.rs` | 107 | new — the crate root and the contract |
| `backend/crates/kala-studio-journal/src/journal.rs` | 857 | new — `StudioSessionJournal` |
| `backend/crates/kala-studio-journal/src/event.rs` | 136 | new — `SessionEvent`, `JournalVerification`, `JournalEventType` |
| `backend/crates/kala-studio-journal/src/row.rs` | 151 | new — reading a stored row and judging it |
| `backend/crates/kala-studio-journal/src/identifier.rs` | 100 | new — the shared safe-identifier rule |
| `backend/crates/kala-studio-journal/src/path.rs` | 166 | new — data-directory resolution and modes |
| `backend/crates/kala-studio-journal/src/error.rs` | 160 | new — the error type and its Python messages |
| `backend/crates/kala-studio-journal/tests/semantics.rs` | 928 | new — 37 tests, the Python unit suite ported |
| `backend/crates/kala-studio-journal/tests/security.rs` | 288 | new — 10 tests |
| `backend/crates/kala-studio-journal/tests/differential.rs` | 582 | new — 6 live interleaved differential tests |
| `backend/crates/kala-studio-journal/tests/corpus_differential.rs` | 466 | new — 1 randomized deterministic corpus differential |
| `backend/crates/kala-studio-journal/tests/perf_shape.rs` | 112 | new — 3 append-shape tests |
| `backend/crates/kala-studio-journal/tests/support/mod.rs` | 175 | new — the Python oracle driver |
| `backend/crates/kala-studio-journal/tests/python_oracle.py` | 263 | new — the JSON-lines Python oracle |

**Not touched:** the root `backend/Cargo.toml` and `backend/Cargo.lock` (both
untouched — the crate has its own `[workspace]`), every other crate, the Python
source and tests, the shared fixtures, `adapters/`, `studio/`. The crate depends on
`kala-artifacts`, `kala-json` and `kala-persistence` **read-only, by path**.

---

## 2. Public API

```rust
use kala_studio_journal::{
    StudioSessionJournal, SessionEvent, JournalVerification, JournalEventType,
    JournalConfig, AppendOptions, JournalDiagnostics, StudioJournalError, ErrorKind,
    JOURNAL_FILE_NAME, ARTIFACT_DIRECTORY_NAME, DEFAULT_AGGREGATE_ID,
    DEFAULT_MAX_PAYLOAD_BYTES, DEFAULT_MAX_ARTIFACT_BYTES, DEFAULT_MAX_PAGE_SIZE,
    DEFAULT_BUSY_TIMEOUT_MS, ContentAddressedArtifactStore,
};
```

### `StudioSessionJournal`

| Member | Signature | Behaviour |
|---|---|---|
| `open` | `(data_dir) -> Result<Self>` | Opens (and creates) the journal with the default bounds. The chain is walked once here. |
| `open_with_config` | `(data_dir, JournalConfig) -> Result<Self>` | The same with explicit bounds. |
| `append` | `(event_type, payload: &CanonicalValue) -> Result<SessionEvent>` | Canonicalises the payload, stores it, returns the event with `sequence`, `digest` (payload SHA-256) and `event_digest` (chain digest). |
| `append_with` | `(event_type, payload, &AppendOptions) -> Result<SessionEvent>` | The same with an explicit aggregate id and occurrence time. |
| `append_json` | `(event_type, payload_json: &str) -> Result<SessionEvent>` | The text form: the text is parsed, then canonicalised. |
| `append_json_with` | `(event_type, payload_json, &AppendOptions) -> Result<SessionEvent>` | The same with explicit optional values. |
| `page` | `(after_sequence: i64, limit: i64) -> Result<Vec<SessionEvent>>` | Events with `sequence > after_sequence`, ascending, capped at `max_page_size`. `ValueError` for a negative cursor or a limit < 1. |
| `get` | `(sequence: i64) -> Result<Option<SessionEvent>>` | One event by sequence. |
| `verify` | `() -> Result<JournalVerification>` | `{ok, event_count, head_digest, first_bad_sequence}`. |
| `head_digest` | `() -> Result<Option<String>>` | The verified chain digest of the newest event. Like the Python property, it walks the whole chain. |
| `put_artifact` | `(data: &[u8]) -> Result<String>` | Stores the bytes in the content-addressed store, returns the SHA-256. Writes **no** event; the caller appends `artifact.stored`. |
| `get_artifact` | `(digest) -> Result<Vec<u8>>` | The stored bytes. |
| `get_artifact_text` | `(digest) -> Result<String>` | The stored bytes as UTF-8 text. |
| `has_artifact` | `(digest) -> Result<bool>` | Whether the object is stored. |
| `data_dir` / `database_path` / `artifact_root` | accessors | `<data_dir>/studio-journal.sqlite3`, `<data_dir>/artifacts`. |
| `max_payload_bytes` / `max_page_size` | accessors | The configured bounds. |
| `diagnostics` | `() -> JournalDiagnostics` | `{chain_walks, fast_path_appends, fallback_appends}`. |
| `close` | `() -> Result<()>` | Closes the database; closing twice is harmless. `Drop` does the same. |

The type is `Send + Sync`, so one instance may be shared by a server thread pool
(pinned by a test).

### `SessionEvent`

`sequence`, `event_type`, `aggregate_id`, `payload` (`Option<CanonicalValue>`),
`payload_json` (the exact canonical text that was hashed), `digest` (payload
SHA-256), `occurred_at` (the exact ISO-8601 UTC string that is part of the chain
preimage), `previous_event_digest`, `event_digest`, plus `timestamp()` (the parsed
aware UTC value).

`payload` is `None` only when the stored text is not a JSON object, which can happen
only if the file was edited outside the Studio; `payload_json` always holds the raw
text.

### `JournalVerification`

`ok`, `event_count`, `head_digest`, `first_bad_sequence` — the four documented keys
of the Python `JournalVerification` mapping.

### `JournalEventType`

`design.intent`, `design.compiled`, `design.measured`, `design.checked`,
`artifact.stored`, `export.completed`, `chat.turn` (ADR-0009 §5). `append` accepts
any safe identifier, so the enum is a convenience, not a gate.

### `StudioJournalError`

One variant per Python failure, each carrying the **Python message verbatim**:

| Variant | Python | Message |
|---|---|---|
| `Closed` | `RuntimeError` | `the studio session journal is closed` |
| `PayloadNotAMapping` | `TypeError` | `event payload must be a mapping` |
| `InvalidIdentifier { field }` | `ValueError` | `event_type must be a safe identifier` |
| `InvalidPayload` | `ValueError` | `value must be a canonical JSON object` |
| `PayloadTooLarge` | `ValueError` | `event payload exceeds the configured size limit` |
| `NotPositive { field }` | `ValueError` | `limit must be positive` |
| `NegativeValue { field }` | `ValueError` | `after_sequence must be nonnegative` |
| `Integrity { sequence }` | `JournalIntegrityError` | `the studio session journal is broken at sequence N; refusing to extend it` |
| `Artifact(..)` | `ValueError` / `FileNotFoundError` | the `kala-artifacts` messages |
| `Json` / `Persistence` / `Database` / `Io` | — | the underlying crate's messages |

`ErrorKind::kind()` maps a failure to the coarse Python class (`type`, `value`,
`integrity`, `not_found`, `closed`), which is what the differential compares.

---

## 3. What is reused (and what is not invented)

- **The chain digest is the core chain digest.** `kala_json::chain_digest` over
  `EventChainFields` — the same eight length-prefixed parts in the same order, the
  same domain (`kala-agent.event-chain.v1.sha256`), the same SHA-256. A Studio
  chain digest is reproducible with the core function without any Studio-specific
  knowledge. `tests/semantics.rs::append_uses_the_core_chain_preimage` recomputes
  the digest independently and compares.
- **The artifact store is `kala_artifacts::ContentAddressedArtifactStore`**, the
  native equivalent of `kala_agent.persistence.artifacts.ContentAddressedArtifactStore`,
  with the same `objects/<ab>/<digest>` layout, the same digest normalisation and
  the same hardened publication.
- **The canonical JSON form is `kala_persistence::canonicalize_json_object`**, the
  very function the core journal uses: sorted keys, no insignificant whitespace,
  ASCII-escaped, no NaN or Infinity, arbitrary-precision integers.
- **The identifier rule** (`^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$` after CPython's
  `str.strip()`) is restated in `src/identifier.rs` and pinned by unit tests.
  `kala-persistence` keeps its copy `pub(crate)`, so it could not be imported; the
  cell named only three accepted crates, and `kala-contracts` was not one of them.
- **Not used:** `KalaApplication`, `persistence.database.Database`,
  `persistence.journal.EventJournal`. The core database is never opened. A test
  asserts that the crate's manifest names no core-application crate.

---

## 4. The byte-compatibility evidence

### 4.1 The same schema

The DDL strings are byte-identical to the Python ones, so `sqlite_master.sql` is
identical whoever creates the file. The differential compares the stored schema of
the two files and asserts equality; the corpus differential compares the schema of
a Python-created file and a Rust-created file.

```
('index', 'studio_events_type_sequence_idx', 'CREATE INDEX studio_events_type_sequence_idx\n    ON studio_events(event_type, sequence)\n    ')
('table', 'studio_events', 'CREATE TABLE studio_events (\n        sequence INTEGER PRIMARY KEY,\n ... \n    )')
```

Both implementations run `PRAGMA journal_mode = WAL` and `PRAGMA synchronous = FULL`
(`synchronous` reads back as `2` = FULL) and neither writes `user_version` or
`application_id`.

### 4.2 The same rows

The differential reads the **raw stored rows** of the shared file through both
implementations and compares them column by column: `sequence`, `event_type`,
`aggregate_id`, `workflow_id`, `run_id`, `payload_json`, `digest`, `occurred_at`,
`previous_event_digest`, `event_digest`. They are equal.

The corpus differential goes further: it runs the same 240-operation corpus in two
separate directories, one written by the native journal and one by the Python
journal, and then compares the two files row by row. The two files hold the same
events with the same chain digests, byte for byte.

### 4.3 The same answers

Every operation of the corpus is answered by both implementations and the answers
are compared field by field, including the failure class. The corpus mixes
appends, pages, gets, verifications, head digests and artifact reads, over a
payload pool of 18 objects (nested objects and arrays, unicode and emoji, big
integers, floats, `-0.0`, `1e-05`, `1e+20`, nulls, booleans) and three aggregate
ids and seven event types. Its seed is a constant of the test file, so a failure
reproduces exactly.

### 4.4 The same verdicts on tampering

The tamper cases are run from both sides and the two implementations must report
the same `first_bad_sequence`:

| Tamper | `first_bad_sequence` | Both |
|---|---|---|
| a payload byte at sequence 2 | 2 | yes |
| the `event_digest` at sequence 3 | 3 | yes |
| a dropped event at sequence 2 | 2 | yes |

A broken chain is refused by both, with the same message
(`the studio session journal is broken at sequence N; refusing to extend it`), and
the history stays readable by both.

### 4.5 The same file under concurrency

One test runs 40 Python appends and 40 native appends **at the same time** against
one file, from two processes, with both journals open. The result is 80 events
with the sequences 1…80, every event linked to the one before it, and both
implementations reporting `ok: true` and the same head digest. The chain is never
forked: the loser of a race re-walks and continues from the real head.

---

## 5. The O(1) append

The chain is walked **once**, when the journal is opened, and the verified head is
cached. An append re-reads only the newest stored row, re-verifies that row against
the digests it records itself, and checks that it is still the cached head. If
anything mismatches — the head was edited, or another instance appended to the same
file — the journal falls back to a full walk and refuses the append when that walk
finds the chain broken.

`StudioSessionJournal::diagnostics()` exposes the counters, so the shape is
*observed* rather than assumed:

| Test | Assertion |
|---|---|
| `appends_after_a_long_history_do_not_walk_the_whole_chain` | after 2,000 events, 100 more appends change `chain_walks` by **0** and `fast_path_appends` by **+100** |
| `a_long_history_costs_no_more_per_append_than_a_short_one` | after 5,000 events, 1,000 more appends change `chain_walks` by **0** |
| `the_first_append_after_an_open_pays_for_one_walk` | opening walks exactly once; the first append uses the cached head |
| `a_second_instance_on_the_same_file_pays_for_one_walk` | an append that finds a foreign head walks exactly once |

### Measured numbers

Measured on this machine, SQLite file on the local disk.

| Measurement | Python | Native |
|---|---|---|
| 2,000 appends on a fresh journal | 0.064 s | 0.043 s (release) |
| appends at a large history | 500 appends at history size 2,000 → **31.1 µs/append** | 500 appends at history size 2,000 → **19.3 µs/append** (release); 1,000 appends at history size 6,000 → 19.2 µs (release) / 91.8 µs (debug) |
| `verify()` on a large journal | 2,500 events → 0.0149 s | 2,500 events → 0.0078 s (release) |

The per-append cost is flat in the history size in both implementations, which is
the point of the cached head.

---

## 6. The detection guarantee (stated, not implied)

| Edit | Detected by | When |
|---|---|---|
| the **head** (newest event) | the next `append` | immediately — the row no longer matches the cached head, the journal falls back to a full walk and refuses the write |
| the **middle** of the chain | the next `verify()` or the next **open** | at the next full walk — only a walk re-derives the digests of the events that follow the edited one |

This is a real, deliberate narrowing, and it is the Python module's behaviour
exactly, not a native invention. It was checked against the live Python: after a
mid-chain edit the Python `append` **also** succeeds (extending the chain as it
stands), its `verify()` reports the break, and the next `append` is refused. The
native implementation does the same, and `tests/security.rs::a_broken_chain_is_never_extended`
pins it.

A broken chain is never extended. A journal found broken on open, or one that
fails a verification, refuses every append until a later verification walks the
chain and finds it intact again.

The chain is tamper-**evident**, not tamper-**proof**: anyone with write access to
the file can rewrite a whole suffix consistently. The head digest must be anchored
outside the database.

---

## 7. Test counts and results

Command lines are the `WAVE.md` / `CONVENTIONS.md` gates, run from
`backend/crates/kala-studio-journal`.

| Gate | Result |
|---|---|
| `cargo fmt --check` | **clean** |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | **no warnings** |
| `cargo test --locked` (debug) | **60 passed, 0 failed** |
| `cargo test --locked --release` | **60 passed, 0 failed** |
| `cargo build --release --locked` | **ok** |
| `cargo check --locked --target x86_64-pc-windows-gnu` | **not completed — see §9** |

Test breakdown (identical in debug and release):

| Suite | Tests | What it covers |
|---|---|---|
| unit (`src/`) | 2 | the identifier pattern and the Python `str.strip()` whitespace set |
| `tests/semantics.rs` | 37 | the Python unit suite ported one for one, plus the validation order, `Send + Sync`, and the closed-journal contract |
| `tests/security.rs` | 10 | owner-only modes, no write outside the data directory, unsafe identifiers, non-positive bounds, digest path traversal, a symlinked artifact root, the broken-chain refusal, the size limits |
| `tests/differential.rs` | 6 | the live interleaved differential (below) |
| `tests/corpus_differential.rs` | 1 | the 240-operation seeded corpus differential |
| `tests/perf_shape.rs` | 3 | the append shape and the measured cost |
| doc-test | 1 | the crate's usage example |

### The differential, in detail

The oracle is the repository's own Python module, driven over JSON lines by
`tests/python_oracle.py`; the process stays alive with the journal open, so it
exercises the same cached-head behaviour the native journal does. The oracle is
spawned with `PYTHONPATH=<repo>/src` and the repository virtual environment
(`$KALA_PYTHON` overrides it).

| Test | What it proves |
|---|---|
| `python_and_rust_write_one_file_with_the_same_bytes` | The two implementations use the same journal file and the same artifact root; three native events are read back identically by Python; two Python events are read back identically by Rust; the native event that follows links to the Python head; the raw rows of the file are identical; the verdicts are identical; the stored schema is identical |
| `both_implementations_detect_the_same_tampering` | A payload tamper, a chain-digest tamper and a dropped event each produce the same `first_bad_sequence` in both; both refuse to extend a broken chain with the same message; the history stays readable |
| `both_implementations_refuse_the_same_invalid_input` | An unsafe event type, a non-mapping payload, an invalid payload and an unknown artifact produce the same failure class and the same message |
| `both_implementations_store_the_same_artifacts` | The same digests, the same bytes, the same `objects/<ab>/<digest>` layout |
| `the_two_implementations_append_concurrently_to_one_file` | 40 native and 40 Python appends at the same time on one file: 80 events, sequences 1…80, every link correct, one chain, one head digest, `ok: true` in both |
| `a_long_native_history_is_still_written_by_python` | After 50 native events the Python implementation extends the same chain and the native side reads the Python event identically |
| `the_corpus_produces_the_same_answers_and_the_same_file` | The seeded 240-operation corpus in two directories: every answer equal, the two files equal row by row, the two schemas equal, the two artifact trees equal |

**Zero unexplained mismatches.** Every difference that remains is listed in §8.

---

## 8. Intentional differences (the complete list)

1. **The payload is a JSON value, not a text.** The Python module takes a Python
   mapping; the native crate takes a `CanonicalValue` (`append`) or JSON text
   (`append_json`, which parses the text first). The Python module has no text
   entry point, so the *text* form has no Python counterpart — the oracle
   reproduces it by calling `json.loads` and then the module, and by routing a
   text that is not JSON through the module's own `_canonical_json`, so the
   failure the oracle reports is the module's own failure.
2. **Duplicate keys in a payload text are refused.** CPython's `json.loads` takes
   the last value of a repeated key; the accepted `kala-json` parser rejects
   duplicate keys, and the native journal therefore reports
   `value must be a canonical JSON object` where the Python *text* path would
   have kept the last value. The Python module's own API cannot receive a
   duplicate key (it takes a mapping), so this is a difference of the text entry
   point only, and the file format is unaffected. The same choice was made by the
   accepted `kala-json` and `kala-persistence` crates.
3. **A stored payload that is not a strict JSON object decodes to `None`.** The
   native decoder uses the strict parser, so a tampered payload that holds `NaN`
   or a repeated key decodes to `None`, where CPython's `json.loads` would return a
   mapping. The raw text is always available as `payload_json`, the *chain* is
   unaffected, and the acceptance decision is the same. Only the convenience
   field differs, and only for a file that was edited outside the Studio.
4. **The native domain is wider than CPython's incidental recursion limit.** The
   accepted `kala-json` parser is iterative and accepts a deeply nested document
   that CPython's recursive `json.loads` refuses with `RecursionError`. This is
   the house position of the accepted crates and is reported, not hidden; the
   corpus deliberately stays shallow.
5. **A stored previous digest that is not 64 lowercase hexadecimal digits is
   treated as "not self-consistent".** The Python module puts the stored string
   into the chain preimage verbatim, so a crafted row could in principle verify
   with a non-hexadecimal previous digest. The native code refuses such a row.
   The difference is **unobservable**: for the fast path to accept a row, the row's
   `event_digest` must still equal the cached head digest, and a changed previous
   digest changes the event digest, so the Python fast path refuses the row as
   well. The walk reports the same `first_bad_sequence` in both.
6. **The exception classes are types, not classes.** The Python module raises
   `TypeError`, `ValueError`, `RuntimeError`, `FileNotFoundError` and
   `JournalIntegrityError`; the native crate raises one `StudioJournalError` enum
   with one variant per condition and the same message. `ErrorKind::kind()` and
   `ErrorKind::python_class()` map a failure back to the Python class.
7. **`JournalVerification` is a struct, not a mapping.** The Python object is
   also a `Mapping`, so `dict(verification)` and `verification["ok"]` work; the
   native struct exposes the same four fields publicly. There is no
   `__getitem__`/`__iter__` in Rust, and none is needed by the callers.
8. **`SessionEvent.as_dict()` has no native counterpart.** The native event is a
   struct with public fields; a JSON view is the caller's job.
9. **`~user` is not expanded.** `Path.expanduser()` expands `~user` on POSIX; the
   native `expand_user` expands `~` and `~/` only, which is what
   `Path.expanduser()` does on Windows. A data directory named `~user` is
   therefore taken literally.
10. **A naive timestamp cannot be passed.** The Python `append` refuses a
    `datetime` without a timezone with `timestamp must include a timezone`; the
    native API takes `chrono::DateTime<Utc>`, so a naive timestamp is not
    expressible.
11. **`put_artifact` takes `&[u8]`.** The Python method raises `TypeError` for a
    non-`bytes` argument; the native signature enforces the same rule at compile
    time, so there is no runtime error and no message to reproduce.

---

## 9. Remaining gaps and honest notes

1. **The Windows cross-check could not be completed here.** The target
   `x86_64-pc-windows-gnu` *is* installed, but the check fails inside
   `libsqlite3-sys`, whose `bundled` build of SQLite needs a C cross-compiler
   (`x86_64-w64-mingw32-gcc`). The wave's own rule is that the user installs
   `mingw-w64-gcc` and the `windows-check` cell runs afterwards. What *was*
   verified here: the platform-conditional code of this crate (`src/path.rs`:
   the `#[cfg(not(unix))]` mode handling and the `#[cfg(windows)]` home
   directory lookup) compiles clean for `x86_64-pc-windows-gnu` with
   `clippy -D warnings`, in an isolated harness. The rest of the crate is
   portable Rust plus `rusqlite`, `chrono` and `parking_lot`, which are already
   cross-checked by the accepted crates.
2. **No runtime cutover claim.** Nothing in the product was switched over. The
   native journal is a library; the `native-design` cell is what wires it into
   `kala-studio`.
3. **The user's real journal was not opened.** The backed-up file
   (`studio-journal.sqlite3.test-pollution-20261019.bak`) is user data and was
   never read. Opening a real-world journal with the native implementation is a
   one-line check for the conductor, and it is the one piece of evidence this
   cell could not gather without breaking the USER DATA rule.
4. **The chain is tamper-evident, not tamper-proof.** Anyone with write access to
   the file can rewrite a whole suffix consistently. The head digest must be
   anchored outside the database — the same caveat the core chain states.
5. **`verify()` is O(n)** and walks every row. It is the explicit "re-check
   everything" entry point, and it refreshes the cached head, so the next append
   is cheap again.
6. **`head_digest()` is O(n)** as well: it calls `verify()`, exactly like the
   Python property, and therefore reports the last *verified* digest rather than
   the cached head. That asymmetry is deliberate — after a mid-chain edit the
   cached head is stale, and a stale digest must not be reported as verified.
7. **The refusal is sticky until a verification.** A journal that is broken when
   it is opened refuses every append until a `verify()` walks the chain and
   finds it intact again.
8. **The cached head lives in memory only.** Reopening the journal re-walks the
   chain, so the guarantee is exactly "detected at the next open or `verify()`".
   No state is persisted outside the database, which keeps the file the single
   source of truth.
9. **A second instance on the same file is handled by re-walking.** Two journals
   writing the same file never fork the chain; the loser of the race sees a head
   that is not its cached head, walks the chain and continues from the real head.
   The cost of that is one full walk, paid only when the file actually changed
   underneath.
10. **Under concurrent appends the wall-clock order of `occurred_at` may differ
    from the sequence order**, because the timestamp is taken before the write
    lock is taken — the Python module's behaviour, reproduced. The chain is
    unaffected (the stored value is what is hashed); only a UI timeline built
    from timestamps could look out of order.
11. **The `page` bound is a limit, not a guarantee.** `page` returns at most
    `min(limit, max_page_size)` events, exactly like Python.
12. **No route, schema or provider work** — out of this cell's scope.
13. **The aggregate id defaults to `"studio"`**, as in Python. A per-design
    aggregation is available through `AppendOptions::with_aggregate_id`.
14. **The Studio journal does not reuse `persistence.database.Database`**, so the
    Studio gets no snapshot/restore/backup helper from the core; that was the
    Python module's deliberate reading of "reuses only the `chain` and
    `artifacts` libraries", and it is kept.

---

## 10. The correctness argument for the parts that are not "just a port"

- **The chain digest is the core chain digest**, recomputed independently in a
  test and compared. The native code never builds a preimage of its own.
- **A stored row is judged by the same rule in both implementations**: the
  payload digest must be the SHA-256 of the stored payload bytes, and the chain
  digest must be the chain digest recomputed over the covered fields the row
  itself records. The native code uses the accepted `kala-json` chain function
  for that recomputation.
- **The walk stops at the first broken event** and reports the first bad
  sequence, exactly like Python. The stored row count travels in the same SQL
  statement as the rows, so `event_count` and the walked rows always describe
  one snapshot — the Python module gets the same guarantee from its single
  `fetchall()`.
- **The native walk streams the rows** instead of materialising them, so a
  journal with a large payload does not have to fit in memory. The trade-off:
  after the first broken event the remaining rows are counted but not judged.
  The Python walk stops reading there; the *result* is identical, because the
  count comes from the same statement and the verdict was already decided.
- **The lock is one mutex over one connection**, so no statement can interleave
  with another thread's transaction. The native type is `Send + Sync`, which
  the Python module achieves with `check_same_thread=False` plus an `RLock`.
- **The write transaction is `BEGIN IMMEDIATE`**, as in Python, so the head
  resolution and the insert are atomic against other *processes* too — which is
  what makes the concurrent differential possible.
