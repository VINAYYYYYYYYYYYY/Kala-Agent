# application-review — independent adversarial review of `kala-application`

Cell: `application-review` of the `rust-backend-20261019` wave. Reviewer
role: independent reviewer (no commit, no push, no delegation, no credential
read, no runtime cutover).

Subject under review: `backend/crates/kala-application` as accepted from
`application-r1` (see `../application-r1/REPORT.md`), against the repository's
own Python root `src/kala_agent/application.py` (3,010 lines) and the Python
tests under `tests/unit`, `tests/security` and `tests/integration`.

## 0. What this review did, and what it did not touch

Added, and **only** these:

| Path | What |
|---|---|
| `backend/crates/kala-application/tests/review_digests.rs` | probe 1 — the digest identities, re-proved without digest normalisation |
| `backend/crates/kala-application/tests/review_revision_commit.rs` | probe 2 — two-phase commit: races, finality, replay, forgery |
| `backend/crates/kala-application/tests/review_checkpoints.rs` | probe 3 — workflow checkpoints and resume after a restart |
| `backend/crates/kala-application/tests/review_cross_read.rs` | probe 4 — cross-read safety of a foreign data directory |
| `backend/crates/kala-application/tests/review_lifecycle.rs` | probe 5 — close semantics and descriptor accounting |
| `backend/crates/kala-application/tests/review_error_precedence.rs` | probe 6 — error precedence and messages on 69 inputs |
| `.mece/waves/rust-backend-20261019/cells/application-review/REVIEW.md` | this file |

`backend/crates/kala-application/src/**` was **not** edited: the `src/` file
sizes and modification times are unchanged from the `application-r1` hand-off
(`application.rs` 155,087 bytes, 04:51). `cargo fmt` was run inside the crate
and `cargo fmt --check` reported diffs **only** in the six new test files, so
no pre-existing file was reformatted. No other crate, root manifest, lockfile,
Python source, fixture, adapter or `studio/**` file was touched. Every probe
wrote only into `tempfile::tempdir()` directories; `/home/v9i/.local/share/kala-agent`
was never opened (every `Settings`/`settings_from_values` call names an
explicit absolute `data_dir`).

The live Python oracle is used the same way the accepted cell used it:
`/home/v9i/Projects/Kala-Agent/.venv/bin/python -I` with the repository `src`
inserted by the probe script itself, so the isolation is real. The Rust library
never invokes Python; the probes drive the two implementations separately and
compare the results.

## 1. Verdict per probe

| # | Probe | Verdict | Evidence |
|---|---|---|---|
| 1 | Digests without normalisation | **PASS** | `tests/review_digests.rs` (2 tests) |
| 2 | Revision commit two-phase: races, finality, replay, forgery | **PASS** | `tests/review_revision_commit.rs` (6 tests) |
| 3 | Workflow checkpoints | **PASS** | `tests/review_checkpoints.rs` (2 tests) |
| 4 | Cross-read safety | **PASS**, one divergence recorded (**D1**) | `tests/review_cross_read.rs` (5 tests) |
| 5 | Close semantics and descriptor leaks | **PASS** | `tests/review_lifecycle.rs` (4 tests) |
| 6 | Error precedence and messages vs Python | **PASS** | `tests/review_error_precedence.rs` (1 test, 69 cases) |

One finding is recorded, **D1**, a low-severity *undeclared* difference in one
error path. It does not weaken any fail-closed property. Details in §3.

## 2. The probes in detail

### Probe 1 — the digests, without normalisation — PASS

`application-r1` normalises every 64-hex digest before comparing the two
traces and then proves the digest algorithm separately for **one** prepared
commit. This probe does not trust that normalisation: it re-derives every
digest from the raw recorded material with an implementation of the Python
algorithm written from `src/kala_agent/application.py`, and it does so for
**five** prepared commits — mechanical with no head, mechanical with a head,
building, wing, and mechanical with explicit parameters.

For each of the five cases the probe asserts, with nothing normalised except
the one declared native package digest:

* `parameters_digest` is **byte-equal** to Python, and is shown to be the
  canonical digest of the parameter dump on each side;
* the planned `sequence`, the planned `revision_id` and the
  `parent_revision_id` are **byte-equal** to Python;
* the **specification** is byte-equal to Python after substituting the native
  package digest with the Python one, and each side's own `spec_digest` is
  re-derived from its own spec;
* the **report artifact bytes** are byte-equal to Python after the same
  substitution, and the artifact digest is shown to be the SHA-256 of the
  stored bytes on each side (the stored bytes are the canonical report JSON
  plus exactly one trailing newline);
* the **state digest** is re-derived on each side from a preimage written
  from `application.py` (`state_preimage`: artifact digests, artifact
  metadata, design head, design id, domain, package digests, parameters
  digest, parent provenance ids, parent revision id, provenance id, spec
  digest, transformation ids) and equals the recorded digest; after the
  substitution the two preimages are byte-identical and the digests are
  byte-equal;
* the **action digest** and the **request identity**
  (`commit-<design>-<state[:16]>-<action[:16]>`) are re-derived the same way
  and are byte-equal to Python after the substitution.

The journal is checked the same way: for every row of the Python-written and
the Rust-written data directories the probe recomputes the payload digest
(SHA-256 of the canonical payload) and the **chain digest** with an
independent implementation of `src/kala_agent/persistence/chain.py` (the
`kala-agent.event-chain.v1.sha256` domain prefix and the eight-byte
length-prefixed parts), and asserts that both equal the stored values and
that every row links to its predecessor. The identity-independent event
payloads and their payload digests are **byte-equal** between the two
implementations (`plan.created`, `workflow.started`, `workflow.created`,
`workflow.status_changed`, `workflow.transition`, `workflow.step_evaluated`,
`workflow.checkpoint`, `workflow.failed`, `workflow.completed`).

Finally, Rust opens the **Python-written** directory and `verify_chain()`
reproduces the Python chain **head digest byte for byte** — the strongest
available statement that the two implementations compute the chain digest
identically.

Verdict: **PASS.** The only difference between the two implementations is the
declared native package identity, and the report's §8.1 describes it accurately.

### Probe 2 — the two-phase revision commit — PASS

**Finality and replay (step-for-step against Python).** Twenty-two scripted
operations are driven through the live Python root and through the Rust root
and the error class family, the code and the message are compared for each:
approve → replay the decision → replay the commit → read the commit → read the
revisions; reject → replay the decision → replay the commit → read the commit
→ **prepare again after the rejection** (the derivation is identical, so the
request identity is identical, and the preparation is refused as already
decided); stale state digest; stale action digest; a design head that moved
after the preparation; an unknown request; a malformed request; a malformed
decision; a decision bound to the wrong workflow. **Every code and message
matches Python.** The only text that differs is the request identity itself,
which embeds the two digests and therefore the declared package identity; that
identity is normalised out of the compared messages and nothing else is.

**Races (threads).**

* Eight threads commit the *same* prepared request: **exactly one** wins; the
  losers report a defined conflict (`checkpoint_not_pending`,
  `revision_head_moved` or `checkpoint_digest_mismatch`), never a panic and
  never a silent second write; exactly one `design_revisions` row and exactly
  one `revision.created` event exist afterwards.
* Six threads each prepare *and* commit the same design: one writer wins the
  compare-and-swap, the history stays linear (`rev-race-linear-r0` only), and
  at most one `revision.commit_conflicted` event is journalled.
* A decision racing a preparation of the same design produces exactly one
  revision.

**A forged pending commit restored from disk.** The probe takes a prepared
commit, edits the `summary` the preparation recorded, **and re-chains the
whole journal** (recomputing every payload digest and every chain digest and
every predecessor link, after dropping the append-only trigger exactly as an
attacker with database access would). The chain still verifies — the forgery
is invisible to the journal — and the decision is then refused with
`checkpoint_digest_mismatch` ("the state prepared for revision commit … has
changed"), writing no revision. The re-derivation of the request identity is
what catches the forgery, and it does so in Rust exactly as in Python.

**Durability.** A prepared commit survives a restart: a second launch reads
the same request identity and the same digests, reports it `pending` and
`committed == false`, and can still commit it with the digests it was reviewed
against.

Verdict: **PASS.**

### Probe 3 — the workflow checkpoints — PASS

Twenty-three scripted operations are compared with Python step for step
(code and message): approving, rejecting and deciding on a workflow that was
never started; the checkpoint list of an unknown workflow (an empty list, not
an error); a stale state digest; a stale action digest; the checkpoint state
after a stale digest (still pending); a malformed decision; a malformed
workflow id; a reserved workflow id; the approval; a second decision after the
approval; a second run after the approval; the revisions; **a foreign
checkpoint identity** (the state digest of one workflow's checkpoint offered
to another workflow's checkpoint); an unknown workflow id; the other
workflow's checkpoint list; the rejection; a decision after the rejection; and
the revisions after the rejection. Every code and message matches Python.

**Resume after a restart from a Python-written data directory.** Python writes
the directory and stops at the checkpoint. Rust opens it, finds the single
pending checkpoint, approves it, drives the workflow to `completed` and writes
`rev-resume-design-r0`. Python then opens the **Rust-written** directory and
reads the same completed workflow and the same approved checkpoint back.

Verdict: **PASS.**

### Probe 4 — cross-read safety — PASS (with D1)

Rust opening a Python-written data directory that cannot be trusted:

| Tamper | Python | Rust | Verdict |
|---|---|---|---|
| a journal row's payload edited (chain not repaired) | refuses the launch, `JournalIntegrityError` | refuses the launch, `application_error` / "the event journal cannot be verified: …" | same decision, fail closed |
| a stored report artifact truncated | refuses the decision, "artifact digest verification failed" | refuses the decision, the same message | identical |
| a stored report artifact removed | refuses the decision, "the report artifact a revision commit was prepared against is not in the store: …" | refuses the decision, the same message | identical |
| a `design_revisions` row with no journalled commit behind it | refuses, `provenance_unavailable` | refuses, the same code and message | identical |
| an event payload above the bounded event-payload size | refuses the listing | refuses the listing, **different message and class** | **D1** |

Two properties were checked beyond the refusal itself:

* the refused Rust launch **writes nothing over the evidence** — the
  `workflow_snapshots` row count is unchanged after a failed launch, and the
  launch fails again the same way (there is deliberately no `Drop` flush);
* nothing is written on the refusal paths either — no `design_revisions` row
  exists after a truncated, removed or forged artifact.

Verdict: **PASS** for the fail-closed requirement ("must refuse, never
silently continue"). **D1** is recorded below.

### Probe 5 — close semantics — PASS

* **Operations after close.** Twenty operations are compared with Python step
  for step. Every operation that requires an open root is refused with
  `application_closed` / "the application is closed" on both sides, and the
  two read-only surfaces that Python does *not* gate (`list_domain_packages`,
  `list_workflow_definitions`) still answer on both sides. The precedence
  inside the refusal set is identical (a malformed identifier is reported
  before the closed-application refusal, exactly as Python orders it).
* **Double close.** `close()` is idempotent: the second and third calls
  return `Ok` and the root keeps reporting itself closed.
* **Descriptor accounting.** The probe counts the descriptors of the process
  that point into its own data directory (scoped, so the measurement stays
  exact while the other tests of the binary run in parallel). Across ten
  open/work/close cycles: 0 before the first open; the database and the
  artifact store while open; the artifact store only after `close()` (the
  database connection is released by `close()`); **0 again after the root is
  dropped**. No descriptor is leaked by an open/close cycle.
* **An injected database belongs to the caller.** With a caller-supplied
  `Database`, `close()` does not close it, and the caller can still use it
  afterwards.

Verdict: **PASS.**

### Probe 6 — error precedence and messages — PASS

One shared table of **69 operations** is written to a file, read by the Python
probe and by the Rust probe, and executed on both sides. For every case the
success/failure verdict, the error code and the error message are compared.
The table deliberately mixes the order of the checks so that the *precedence*
is compared and not only the outcome.

Coverage: identifier validation (empty, embedded space, leading dash, slash,
non-ASCII, a control character, 64 characters, 65 characters, 148 characters,
a trimmed identifier), domain validation (empty, unknown, embedded space,
upper case, trailing dot), reserved identifiers (both kernel-state aggregate
names, the `kernel-state:` namespace, a namespaced snapshot name), parameter
validation (not an object, a string, an unknown field, a wrong type, a
missing field, an out-of-range value, a foreign domain's field, 4,097 entries
against the bounded entry count), limits (`0`, `-1`, `1001`, the maximum),
`after_sequence` (`-1`), event filters (a malformed event type, a malformed
aggregate id, a reserved workflow id), text bounds (an empty summary, a
whitespace-only summary, a 540-character summary, an empty reason), revision
lookups (an unknown revision, an unknown parent, an unknown prepared commit),
decision validation (`maybe`, `APPROVE`, a trimmed `" approve "`), actor
validation (malformed, 65 characters), stale digests, and eight explicit
precedence cases (`design_id` before the domain, the domain before
`design_id`, `request_id` before the decision, `after_sequence` before
`limit`, `design_id` before `limit`, the decision before the actor, the
workflow id before the decision, and the request id before the decision).

**All 69 cases match Python exactly** in verdict, code and message. The single
normalisation is the request identity inside the compared text, which embeds
the declared native package digest.

The `trimmed_decision` case is worth calling out: Python trims the decision
before validating it, so `" approve "` is a valid decision and it approves the
prepared commit — the Rust root does exactly the same. The case is placed last
in the table so that it does not mask the stale-digest cases.

Verdict: **PASS.**

## 3. Findings

### D1 — `list_events` on an oversized event payload: undeclared message and class difference (low severity, non-blocking)

**Where.** `backend/crates/kala-application/src/application.rs`, `fn
event_view` (the `kala_json::canonical_dumps` failure is wrapped as
`ApplicationError::application(format!("the event payload cannot be
canonicalized as JSON: {error}"))`).

**Minimal repro.** `tests/review_cross_read.rs::an_oversized_event_payload_is_reported_the_same_way_on_both_sides`.
Append one well-formed, correctly chained event whose payload exceeds the
bounded event-payload size (65,536 bytes) to a data directory, then call
`list_events`.

**Observed.**

| | Python | Rust |
|---|---|---|
| decision | refuses | refuses |
| exception | `JsonEnvelopeError` (a `RuntimeError`/`ValueError`, **not** an `ApplicationError`) | `ApplicationError` |
| `code` | none | `application_error` |
| message | `JSON string exceeds the bounded length` | `the event payload cannot be canonicalized as JSON: JSON string exceeds the bounded length` |

**Why it matters.** The Python text is *contained* in the Rust text, so a
caller that matches the reason still finds it, but the message is not
byte-identical. More concretely, at the HTTP boundary the two differ: Python
raises a non-`ApplicationError`, so `handle_unexpected_error` answers
**500 `internal_error`**; the Rust `application_error` is mapped by
`kala_api::errors` to **400** with the raw message in the body. A caller would
see a different status for the same corrupt directory.

**Why it is not blocking.** Both sides fail closed — the listing is refused,
nothing is written, and nothing is silently continued. The condition needs a
foreign writer that stored an event payload above the bound: every event the
root writes itself is far below 65,536 bytes. In the reference flow the
largest payload the root writes is `revision.prepared` at about 2.4 kB, and
`revision.created` is about 1.2 kB. The underlying condition and the
underlying reason text are identical on both sides; only the wrapper
differs.

**Why it is a finding anyway.** `../application-r1/REPORT.md` §8 declares
nine intentional differences and none of them covers this path. §8.9 claims
that the port "keeps the same behaviour (`false`) and the same error
precedence" for the event-payload bound; the `payload_omitted` flag is
indeed never set, but the *error* path — the only reachable one — carries a
different message and a different class, and that is not declared.

**Suggested fix (one line, in `src/`, which this review did not touch).**
Either
* return the underlying message unwrapped, so the refusal reads
  `JSON string exceeds the bounded length`, or
* keep the prefix and add the difference to §8 of the report as a tenth
  declared difference.

The same wrapping exists in `src/value.rs::canonical_dumps`
("value cannot be canonicalized as JSON: {error}"). It is reachable there
only for the report payload (524,288 bytes) and the recorded budget state
(1 MiB); neither can be pushed over its bound through the public API, because
the parameter model bounds the report content and the budget state is a
counter set.

### No other findings

No other defect was found in the six probed areas. Specifically, the following
were checked and are clean:

* the digest algorithm, the digest inputs and the digest *structure* are
  identical to Python's, for every domain and with and without a design head;
* the request identity is derived from the two digests in both
  implementations;
* the journal chain digest is byte-identical, and the two implementations
  verify each other's chains and reproduce each other's chain head;
* a preparation is single-use, a decision is final, a replay writes nothing,
  a stale digest writes nothing, a moved head writes nothing, a fork writes
  nothing, and a forged preparation writes nothing;
* concurrent writers produce exactly one revision and a linear history;
* a checkpoint is final once decided, a foreign checkpoint identity is
  refused, and a checkpoint survives a restart from a foreign data directory;
* a tampered journal row, a truncated artifact, a removed artifact and an
  unbacked revision are all refused, and a refused launch writes nothing;
* `close()` is idempotent, refuses further work with the Python error and
  leaks no descriptor;
* 69 invalid inputs produce the Python verdict, code and message.

## 4. Counts and gates

### Tests

| Suite | Tests |
|---|---:|
| `tests/cross_read.rs` (accepted) | 2 |
| `tests/differential.rs` (accepted) | 3 |
| `tests/public_api.rs` (accepted) | 5 |
| `tests/security.rs` (accepted) | 23 |
| `tests/review_digests.rs` (this review) | 2 |
| `tests/review_revision_commit.rs` (this review) | 6 |
| `tests/review_checkpoints.rs` (this review) | 2 |
| `tests/review_cross_read.rs` (this review) | 5 |
| `tests/review_lifecycle.rs` (this review) | 4 |
| `tests/review_error_precedence.rs` (this review) | 1 |
| **total** | **53** |

All 53 pass, 0 failed, 0 ignored, in debug and in release. Three consecutive
debug runs produced the same result, so the thread races are not flaky.

Comparison volume in this review: 69 error/precedence cases, 22 two-phase
commit steps, 23 checkpoint steps, 20 post-close operations, 5 prepared
commits re-derived digest by digest, 2 journal chains recomputed row by row.

### Gates (run in `backend/crates/kala-application`)

| Gate | Result |
|---|---|
| `cargo fmt --check` | pass (after formatting the six new test files) |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | pass |
| `cargo test --locked` | 53 passed, 0 failed, 0 ignored |
| `cargo test --locked --release` | 53 passed, 0 failed, 0 ignored |
| `cargo build --locked --release` | pass |
| `cargo check --locked --target x86_64-pc-windows-gnu` | **cannot run**: the target is installed but the mingw C compiler is absent (`x86_64-w64-mingw32-gcc` not found, so `libsqlite3-sys`/`ring` build scripts fail). This is a missing cross toolchain, not a code error. No Windows runtime claim is made. |

### Honest limitations of this review

* The live differential needs `/home/v9i/Projects/Kala-Agent/.venv/bin/python`
  (override with `KALA_PYTHON`); the Rust library itself has no Python
  dependency.
* No runtime cutover, packaging, Windows execution, CLI or HTTP layer was
  exercised, and none is claimed. The HTTP-status consequence of **D1** is
  read from `src/kala_agent/api/errors.py` and
  `backend/crates/kala-api/src/errors.rs`, not measured over a socket.
* `kala-api` was under concurrent editing by another cell during this run;
  its transient compile states were waited out, never edited. The gates above
  were re-run against the state that compiled.
* The reference flow never exercises a tool call or a model completion; the
  executor and the gateway are composed and exposed but not driven by the
  application script. That gap is inherited from `application-r1` and is
  outside the six probed areas.

## 5. Recommendation

**ACCEPT.**

All six probes pass. The composition root is faithful to the Python root in
every area this review could reach: the digest identities outside the declared
native package boundary, the two-phase commit's single-use guarantee under
races and replays, the checkpoint finality and the resume path, the
fail-closed handling of a foreign or corrupt data directory, the close
semantics and the descriptor accounting, and the error precedence and
messages on 69 invalid inputs.

**D1** is a low-severity, *undeclared* difference in one error path: the
refusal for an oversized event payload carries a prefixed message and the
base `application_error` code where Python carries a bare
`JsonEnvelopeError` message and no code. It is not a security defect — both
sides refuse, and the underlying reason text is identical — and it is
reachable only from a foreign writer that stored an event payload above the
bound. The owner should either make the one-line change in
`src/application.rs::event_view` or declare the difference in §8 of
`../application-r1/REPORT.md`; it does not hold the cell back.
