# A3 — Studio session journal and artifacts

Cell: `a3-journal`. Spec: ADR-0009 §5 and acceptance criterion 5.
Status: **done, library level**. No commit, no push, no delegation, no credential reads.

## Files

| File | Lines | Note |
|---|---|---|
| `src/kala_agent/studio/session_journal.py` | 700 | new — the Studio journal + artifact store |
| `tests/unit/studio_design/test_session_journal.py` | 431 | new — 25 tests |

Nothing else was touched. `src/kala_agent/studio/design/**` (A1/A2) and
`tests/unit/studio_design/test_compile_ocp.py`, `test_measure.py`,
`test_drawing.py` (A1/A2) were read only for context, never edited.

`tests/unit/studio_design/__init__.py` was **not** created: `tests/unit/` has no
`__init__.py`, the repo convention is package-less test directories, and no
basename in `tests/unit/studio_design/` collides, so the layout does not need
one. A1 and A2 landed their files in the same directory without one and the
suite collects cleanly.

## Public interface

```python
from kala_agent.studio.session_journal import (
    StudioSessionJournal, SessionEvent, JournalVerification, JournalEventType,
    JOURNAL_FILE_NAME, ARTIFACT_DIRECTORY_NAME, DEFAULT_AGGREGATE_ID,
    DEFAULT_MAX_PAYLOAD_BYTES, DEFAULT_MAX_ARTIFACT_BYTES,
    DEFAULT_MAX_PAGE_SIZE, DEFAULT_BUSY_TIMEOUT_MS,
)
```

### `StudioSessionJournal(data_dir: Path, *, max_payload_bytes=262144, max_artifact_bytes=67108864, max_page_size=500, busy_timeout_ms=5000)`

| Member | Signature | Behaviour |
|---|---|---|
| `append` | `append(event_type: str \| JournalEventType, payload: Mapping[str, object], *, aggregate_id="studio", occurred_at: datetime \| None = None) -> SessionEvent` | Canonicalises the payload, stores it, returns the event with `sequence`, `digest` (payload SHA-256) and `event_digest` (chain digest). Raises `ValueError` for an unsafe event type, a non-JSON payload, or a payload over `max_payload_bytes`; raises `JournalIntegrityError` when the chain it would extend is already broken. |
| `page` | `page(after_sequence: int = 0, limit: int = 500) -> tuple[SessionEvent, ...]` | Events with `sequence > after_sequence`, ascending, capped at `max_page_size`. `ValueError` for a negative cursor or a limit < 1, `TypeError` for a non-int. |
| `get` | `get(sequence: int) -> SessionEvent \| None` | One event by sequence. |
| `verify` | `verify() -> JournalVerification` | `{ok, event_count, head_digest, first_bad_sequence}`. |
| `head_digest` | property → `str \| None` | The verified chain digest of the newest event. |
| `put_artifact` | `put_artifact(data: bytes) -> str` | Stores the bytes in the content-addressed store, returns the sha256. Writes **no** event; the caller appends `artifact.stored`. |
| `get_artifact` | `get_artifact(digest: str) -> bytes` | The stored bytes; `FileNotFoundError` when absent. |
| `get_artifact_text` | `get_artifact_text(digest: str) -> str` | The stored bytes as UTF-8 text. |
| `has_artifact` | `has_artifact(digest: str) -> bool` | Whether the object is stored. |
| `data_dir` / `database_path` / `artifact_root` | properties | `<data_dir>/studio-journal.sqlite3`, `<data_dir>/artifacts`. |
| `max_payload_bytes` / `max_page_size` | properties | The configured bounds. |
| `close()` / `with ... as journal` | — | Closes the database; closing twice is harmless. |

### `SessionEvent` (frozen dataclass)

`sequence`, `event_type`, `aggregate_id`, `payload` (`Mapping[str, object] | None`),
`payload_json` (the exact canonical text that was hashed), `digest` (payload
sha256), `occurred_at` (the exact ISO-8601 UTC string that is part of the chain
preimage), `previous_event_digest`, `event_digest`, plus a `timestamp` property
(parsed UTC `datetime`) and `as_dict()`.

`payload` is `None` only when the stored text is not a JSON object, which can
happen only if the file was edited outside the Studio; `payload_json` always
holds the raw bytes.

### `JournalVerification` (frozen dataclass, also a `Mapping[str, object]`)

`ok`, `event_count`, `head_digest`, `first_bad_sequence`, plus `as_dict()`.
`dict(verification)` and `verification["ok"]` both work, so the API layer can
hand it straight to the JSON encoder.

### `JournalEventType` (`StrEnum`)

`design.intent`, `design.compiled`, `design.measured`, `design.checked`,
`artifact.stored`, `export.completed`, `chat.turn` (ADR-0009 §5). `append`
accepts any safe identifier, so the enum is a convenience, not a gate.

## What is reused (and what is not invented)

- **Hash chain**: `kala_agent.persistence.chain` — `chain_digest`,
  `EventChainFields`, `chain_fields_from_row`, `is_chain_digest`,
  `CHAIN_COLUMNS`, `GENESIS_PREVIOUS_DIGEST`, `JournalIntegrityError`. The
  journal stores exactly the covered fields the core chain covers, in the same
  table shape, so a Studio chain digest is recomputable with the core
  function. `test_append_uses_the_core_chain_preimage` recomputes the digest
  independently with `chain_digest(EventChainFields(...))` and compares.
- **Artifacts**: `kala_agent.persistence.artifacts.ContentAddressedArtifactStore`
  (own root under the Studio data dir, own size bound).
- **Canonical JSON**: `kala_agent.contracts.CanonicalJsonObject`, the very
  validator `kala_agent.persistence.database` uses for the core journal
  (sorted keys, no insignificant whitespace, ASCII-escaped, no NaN/Infinity,
  no duplicate keys). No second canonicalisation scheme was added.
- **Identifiers**: `kala_agent.contracts.Identifier`, the core identifier rule.
- **Not used**: `KalaApplication`, `kala_agent.persistence.database.Database`,
  `kala_agent.persistence.journal.EventJournal`. A test asserts by AST that
  the module imports nothing from `kala_agent.application`, `kala_agent.runtime`
  or `kala_agent.tools`.

## Design decisions

- **Studio-owned storage.** One SQLite file at `<data_dir>/studio-journal.sqlite3`
  and one artifact root at `<data_dir>/artifacts/`, both in the caller-supplied
  data directory. The core Kala database is never opened.
- **Thread safety for a FastAPI thread pool.** One connection
  (`check_same_thread=False`) guarded by one re-entrant lock, so a statement
  can never interleave with another thread's transaction. WAL +
  `synchronous = FULL`; writes run in `BEGIN IMMEDIATE`.
- **Fail closed on write.** Every append re-walks the whole chain inside the
  write transaction before extending it, so a journal edited outside the
  Studio cannot be grown into a second, inconsistent history — the append
  raises `JournalIntegrityError` instead.
- **Reads never raise on a broken chain.** `page()`/`get()` return the stored
  events even when the chain is broken, because the UI has to show the history
  *and* report the chain verdict separately (ADR §7: the `HASH-CHAIN OK` stamp
  shows only when `/journal/verify` reports `ok: true`). `verify()` is the
  authority on integrity.
- **No append-only SQL triggers.** The core journal has them; the Studio
  journal deliberately does not, because the tamper test must be able to
  mutate a stored payload byte through raw SQL, and because the chain is the
  tamper-evidence mechanism. This is the same trade the core chain makes
  (tamper-evident, not tamper-proof).
- **Bounded everywhere.** Canonical payload ≤ 256 KiB, artifact ≤ 64 MiB,
  page ≤ 500 rows. Exceeding a bound raises and writes nothing.
- **Timestamps.** UTC, ISO-8601, microseconds, `Z` suffix — the same form the
  core journal stores. The exact string is kept on the event because it is
  part of the chain preimage.

## Gate results

Command lines are the WAVE.md gates.

| Gate | Result |
|---|---|
| `.venv/bin/python -m pytest tests/unit/studio_design/test_session_journal.py -q` | **25 passed** in 0.29 s |
| `.venv/bin/ruff check src/kala_agent/studio/session_journal.py tests/unit/studio_design/test_session_journal.py` | **All checks passed** |
| `.venv/bin/ruff format --check src/kala_agent/studio/session_journal.py tests/unit/studio_design/test_session_journal.py` | **2 files already formatted** |
| `.venv/bin/mypy src/kala_agent/studio/session_journal.py` | **Success: no issues found in 1 source file** (strict, project config) |
| `.venv/bin/mypy tests/unit/studio_design/test_session_journal.py` | **Blocked by a pre-existing environment error** — see below. Same file with `--python-version 3.14`: **Success: no issues found in 2 source files** (module + test together) |
| `.venv/bin/python -m pytest tests/unit -q` (full suite) | **1352 passed, 1 warning** in 6.2 s (the count moves as A1/A2 land their tests; my 25 are always in it) |

The single warning is pre-existing and unrelated:
`StarletteDeprecationWarning: Using httpx with starlette.testclient is
deprecated` raised from `fastapi/testclient.py`.

### The mypy blocker is pre-existing, not mine

`pyproject.toml` pins `python_version = "3.11"`, but the installed numpy
stubs use a 3.12+ `type` statement, so any mypy run that pulls numpy's stubs
fails with:

```
.venv/lib/python3.14/site-packages/numpy/__init__.pyi:737: error: Type
statement is only supported in Python 3.12 and greater  [syntax]
```

It is triggered by `import pytest` alone and reproduces on files nobody in
this wave touched (`tests/unit/test_studio_step.py`, and A1's
`tests/unit/studio_design/test_compile_ocp.py`). `pyproject.toml` is not my
file, so I did not change it. With `--python-version 3.14` (matching the
3.14 venv) both of my files type-check clean, so my code itself is strict-clean.

## Test coverage (25 tests)

- **append / ordering**: sequence and digest returned; canonical payload
  stored; the digest is the core chain digest recomputed with
  `chain_digest(EventChainFields(...))`; non-JSON payload and unsafe event
  type refused.
- **page**: ascending order after a cursor; cursor/`limit` validation; the
  page is clamped to `max_page_size`.
- **verify — ok chain**: `ok=True`, `event_count`, `head_digest` equal to the
  last event's chain digest, `first_bad_sequence is None`; the empty journal
  verifies.
- **verify — tamper detection**: a raw-SQL mutation of one payload byte at
  sequence 2 gives `ok=False` and `first_bad_sequence == 2`; a mutated
  `event_digest` gives `first_bad_sequence == 2`; a deleted event gives
  `first_bad_sequence == 2` (the missing sequence). A broken journal is not
  extended (`JournalIntegrityError`) and is still readable.
- **artifacts**: round trip returns the sha256 and the same bytes; the same
  bytes are stored once; an unknown digest raises `FileNotFoundError`; the
  size limit is enforced.
- **limits**: an over-sized payload raises and writes nothing; the artifact
  size limit is enforced.
- **placement**: the journal and the artifact root live under the supplied
  data directory; the module imports nothing from the core application.
- **concurrency**: 8 threads × 10 appends give 80 events with sequences
  1…80 and `ok=True`; 4 writers plus 2 concurrent readers (page + verify)
  finish with an intact 40-event chain.

## Gaps and honest notes

1. **Criterion 5 is covered at library level only.** `append`/`page`/
   `verify`/tamper detection are exercised against the module; the ADR's
   `/journal` and `/journal/verify` routes are B1's work, and the UI's
   `HASH-CHAIN OK` stamp is C1's.
2. **`put_artifact` writes no event.** The caller must append
   `artifact.stored` with the returned sha256. This is documented in the
   module docstring, but B1 has to remember it or the STEP blob will not be
   tied to the turn that produced it.
3. **The chain is tamper-evident, not tamper-proof.** Anyone with write access
   to the file can rewrite a whole suffix consistently. The head digest must
   be anchored outside the database — the same caveat the core chain states.
4. **`verify()` semantics are slightly asymmetric on purpose**: `event_count`
   is the number of stored rows (broken ones included) while `head_digest` is
   the last *verified* digest. Both are documented on the class.
5. **Under concurrent appends the wall-clock order of `occurred_at` may
   differ from the sequence order**, because the timestamp is taken before the
   write lock is taken. The chain is unaffected (the stored value is what is
   hashed); only a UI timeline built from timestamps could look out of order.
6. **No route, schema, or provider work** — out of this cell's scope.
7. **The `aggregate_id` defaults to the constant `"studio"`.** Per-design
   aggregation is available through the `aggregate_id` keyword, but B1 has to
   pass it if the UI wants to group events by design.
8. **The Studio journal does not reuse `persistence.database.Database`**, so
   the Studio gets no snapshot/restore/backup helper from the core; that was
   a deliberate reading of "reuses only the `chain` and `artifacts` libraries".

---

# Repair 1 — the append cost (Opus review finding)

**Finding.** `SessionJournal.append` called `self._walk_chain(connection)` on
every append, re-hashing every stored event. Appends were O(n) and a
session's total cost was O(n²). The journal persists on disk across
sessions, so the degradation was unbounded.

**Status: fixed.** Only `src/kala_agent/studio/session_journal.py` and
`tests/unit/studio_design/test_session_journal.py` were edited. No commit, no
push, no delegation, no credential reads.

| File | Lines | Change |
|---|---|---|
| `src/kala_agent/studio/session_journal.py` | 700 → 810 | cached head, O(1) append path, new docstring guarantee |
| `tests/unit/studio_design/test_session_journal.py` | 431 → 635 | 25 → 31 tests (6 new) |

## What changed

### 1. The chain is walked once, on open

`__init__` now calls `_load_chain_state()`, which walks the full chain and
caches the **verified head**: `_head_sequence` (the sequence of the last
event that verified) and `_head_digest` (its chain digest), plus
`_first_bad_sequence` when the walk finds a break. A journal that is opened
on a broken chain records the break and refuses every append, exactly as
before.

### 2. An append verifies one row, not the chain

Under the write lock, `append` calls `_resolve_append_head()`:

- The **fast path** reads *only* the newest stored row
  (`SELECT … ORDER BY sequence DESC LIMIT 1`), re-verifies that one row
  against the digests the row records itself — its payload digest must be
  the SHA-256 of the payload text it stores, and its chain digest must be
  the chain digest recomputed over the covered fields it stores, including
  its stored previous digest — and then checks that the row is still the
  cached head (same sequence, same `event_digest`). If it is, the new event
  links to the cached head.
- The **fallback** is taken whenever anything mismatches (the head was
  edited, or another instance appended to the same file): the journal walks
  the whole chain, refreshes the cache, and refuses the append with
  `JournalIntegrityError` if the walk finds the chain broken.

`_row_is_self_consistent()` is now shared by the fast path and the walk, so
both judge a row by the same rule.

### 3. `verify()` still walks everything

`verify()` walks the full chain and refreshes the cache, so it is the
authority on integrity and the way to re-sync after an external change.

### 4. The guarantee is documented precisely

The module docstring now states the detection windows in full (see
"Gaps" below for the honest wording of the trade-off).

## The detection guarantee (now documented in the module docstring)

| Edit | Detected by | When |
|---|---|---|
| The **head** (newest event) | `append` | immediately — the row no longer matches the cached head, the journal falls back to a full walk and refuses the write |
| The **middle** of the chain | `verify()` or the next **open** | at the next full walk — only a walk re-derives the digests of the events that follow the edited one |

A broken chain is never extended: the append fails closed. A journal found
broken on open, or one that fails a verification, refuses every append until
a later verification walks the chain and finds it intact again.

**This is a real, deliberate weakening of the old behaviour, and it is the
price of not walking the chain on every write.** Under the old code an
append always walked the chain, so *any* edit was refused at the next
append. Under the new code an edit in the middle of the chain is refused at
the next `verify()` or the next open, but an append made in between still
extends the chain as it stands (the newest row still verifies against
itself, so the fast path cannot see the break). The chain stays broken and
is reported as broken; it is never silently repaired. The module docstring
says exactly this.

## Tests added (6)

| Test | What it pins |
|---|---|
| `test_appends_after_a_long_history_do_not_walk_the_whole_chain` | **Performance shape.** After 2000 events, a spy on `_walk_chain` records **zero** calls across the next 100 appends. The spy is installed on the instance, so a full walk would be counted. |
| `test_a_tampered_head_is_detected_at_the_next_append` | **Head tamper at append.** Editing the newest payload makes the next `append` raise `JournalIntegrityError`; `verify()` then reports `first_bad_sequence == 2`. |
| `test_a_tampered_middle_is_detected_by_verify_and_on_reopen` | **Mid-chain tamper.** `verify()` reports `first_bad_sequence == 3`; after a reopen the break is still reported and appends are refused. |
| `test_a_second_instance_on_the_same_file_extends_the_chain` | **External append.** A second `StudioSessionJournal` on the same file appends; the first one re-walks (the spy records the walk) and continues at sequence 3 with the right `previous_event_digest`. The chain verifies `ok` with 3 events — a correct extension, never a fork. |
| `test_a_broken_chain_is_not_extended_after_a_reopen` | A journal opened on a broken chain refuses every write, and the history stays readable. |
| `test_a_repaired_chain_is_writable_again_after_a_verification` | The refusal is lifted by a later successful `verify()`, as the docstring claims. |

**The performance test is a genuine regression test.** Run against the old
append behaviour (the old full walk re-implemented as a patched
`_resolve_append_head`), it fails; the other five pass, because they pin
correctness that the new code must keep.

## Measured numbers

All measured on this machine, `.venv/bin/python` (CPython 3.14.8), SQLite
file on the local disk.

| Measurement | Value |
|---|---|
| 2000 appends on a fresh journal | **0.065 s** |
| 500 appends at history size 2000 | **0.0159 s** → **32 µs/append** |
| `verify()` on a 2100-event journal | **0.012 s** |
| The six new tests, old behaviour | **11.81 s** (1 failed) |
| The same six tests, new behaviour | **0.34 s** |

The per-append cost is flat in the history size, which is the point: the
journal no longer degrades as it grows.

## Gate results (repair 1)

Command lines are the WAVE.md gates.

| Gate | Result |
|---|---|
| `.venv/bin/python -m pytest tests/unit/studio_design/test_session_journal.py -q` | **31 passed** in 0.41 s |
| `.venv/bin/ruff check src/kala_agent/studio/session_journal.py tests/unit/studio_design/test_session_journal.py` | **All checks passed** |
| `.venv/bin/ruff format --check src/kala_agent/studio/session_journal.py tests/unit/studio_design/test_session_journal.py` | **2 files already formatted** |
| `.venv/bin/mypy src/kala_agent/studio/session_journal.py` | **Success: no issues found in 1 source file** (strict, project config) |
| `.venv/bin/mypy tests/unit/studio_design/test_session_journal.py` | **Blocked by the same pre-existing environment error** as in the first round (numpy stubs use a 3.12+ `type` statement while `pyproject.toml` pins `python_version = "3.11"`). It reproduces on files nobody in this wave touched, e.g. `tests/unit/test_studio_step.py`. With `MYPYPATH=src .venv/bin/mypy --python-version 3.14` on the test file alone: **Success: no issues found in 1 source file**; with both files: **Success: no issues found in 2 source files**. |
| `.venv/bin/python -m pytest tests/unit -q` (full suite) | **1358 passed, 1 warning** in 6.25 s (1352 before this repair + the 6 new tests) |

The single warning is the pre-existing
`StarletteDeprecationWarning: Using httpx with starlette.testclient is
deprecated` raised from `fastapi/testclient.py`.

## Gaps and honest notes (repair 1)

1. **The append is O(1) and no longer a full-chain audit.** A mid-chain edit
   made while the journal is open is found at the next `verify()` or the next
   open, not at the next append. This is the documented, intended trade-off;
   the module docstring states it in those words.
2. **`verify()` is still O(n)** and still walks every row. It is now the
   explicit "re-check everything" entry point, and it refreshes the cached
   head, so the next append is cheap again.
3. **`head_digest` still calls `verify()`**, so it is O(n) and it still
   reports the *last verified* digest rather than the cached head. That
   asymmetry is deliberate: after a mid-chain edit the cached head is stale
   (it points at an event that no longer verifies), and `head_digest` must
   not report a stale digest as if it were verified.
4. **The cached head lives in memory only.** Reopening the journal re-walks
   the chain, so the guarantee is exactly "detected at the next open or
   verify()". No state is persisted outside the database, which keeps the
   file the single source of truth.
5. **A second instance on the same file is handled by re-walking.** Two
   journals writing the same file never fork the chain: the loser of the
   race sees a head that is not its cached head, walks the chain, and
   continues from the real head. The cost of that is one full walk, which is
   paid only when the file actually changed underneath.
6. **The refusal is sticky until a verification.** A journal that is broken
   when it is opened refuses every append until a `verify()` walks the chain
   and finds it intact again. `verify()` is cheap to call and is the
   documented authority, but B1 should know that "fix the file, then
   append" is not enough on its own.
7. **The public interface did not change.** No new method, property, or
   constant was added; B1 and C1 need no adaptation. The cached head is
   private (`_head_sequence`, `_head_digest`, `_first_bad_sequence`).
