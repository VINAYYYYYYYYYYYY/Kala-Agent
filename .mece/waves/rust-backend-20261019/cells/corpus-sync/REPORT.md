# corpus-sync — the corpus now says how its own bodies may be compared

**Cell:** `corpus-sync` of the `rust-backend-20261019` wave. **Worker:**
`corpus-sync` (Rust wave, conducted by Opus). **Date:** 2026-10-20.

**Status: the replay is green — 56 cases, 0 mismatches — the corpus carries
its own comparison contract, the `kala-studio` suite is green with and without
`--all-features`, and the 268 / 292 / 20 differentials are unchanged at 0
mismatches.** No commit, push, deployment, delegation or credential read was
performed, and `/home/v9i/.local/share/kala-agent` was never opened, read,
written, listed or named in a command.

---

## 1. What this cell changed, and what it did not

Edited (this cell's own paths):

| Path | Change |
|---|---|
| `backend/compat/generate_corpus.py` | The body-comparison contract: per-case `body_comparison`, the declared volatile paths, the host-dependent mask for the CAD probe, and the manifest's new exclusion. |
| `backend/fixtures/corpus/v1/**` | The regenerated corpus (12 files). The corpus digest moved from `9993939b…` to `53dcb9da…`. |
| `backend/crates/kala-studio/tests/replay_binary_corpus.py` | The harness now honours the corpus's contract instead of comparing raw bytes, and compares the host-probe case against the **current** host probe. |
| `backend/crates/kala-studio/tests/corpus_fixture.rs` | The regenerated manifest (56 cases, 15 routes, `corpus_sha256`) and the 56-case replay. |
| `.mece/waves/rust-backend-20261019/cells/corpus-sync/REPORT.md` | This file. |

**No product code changed.** `src/kala_agent/**`, every Rust crate's `src/**`,
`studio/**`, `adapters/**` and `tests/**` are untouched. The only Rust code
in this cell is in `backend/crates/kala-studio/tests/`.

`backend/compat/corpuslib/**` and `backend/compat/self_check.py` are also
untouched: they are not this cell's paths, and the contract was implemented
in the generator entry point instead (see §3).

---

## 2. The diagnosis, in one page

`replay_binary_corpus.py` reported **56 cases, 5 mismatches**. All five were
in the four cases that `compat-drift` added, and none was a regression in the
Rust front door:

| Case | The mismatch | The real cause |
|---|---|---|
| `studio.cad_providers` | body, and `content-length 542 != 783` | The corpus recorded **this host's CAD probe at generation time** (FreeCAD unavailable, 783 bytes). The replay's worker probes the same host and finds FreeCAD 1.1.4 (542 bytes). The corpus marked the case `body_deterministic: true`, which was wrong: ADR-0011 makes the probe a fact about the host. |
| `studio.design.l_bracket` | body | The corpus itself marks the case `body_deterministic: false` (the OCP STEP export embeds the wall clock, and `journal_head` binds it), but the harness compared the raw bytes. |
| `studio.journal.page` | body | Same: `body_deterministic: false`. The chain digests bind the STEP digest. |
| `studio.journal.verify` | body | Same: `body_deterministic: false`; the chain head binds the STEP digest. |

Two further facts the diagnosis turned up, both of which the fix had to take
into account:

1. **The journal's `occurred_at` is a wall clock in production.** The corpus
   freezes it with a fixture-only fixed clock (its own
   `nondeterministic_fields` says so: *"deterministic fixed clock injected
   into `kala_agent.studio.session_journal` for fixture generation; production
   uses the wall clock"*). A running service therefore reports a different
   timestamp for the same event, and the harness must not compare the value.
2. **The corpus's recorded CAD probe was itself an artefact of the
   generation environment.** Regenerating in this cell's environment finds
   FreeCAD 1.1.4 available, while the checked-in corpus recorded it
   unavailable. The recorded value was a fact about the generator's
   environment, not about the product.

---

## 3. The fix: the corpus declares how each body may be compared

### 3.1 The corpus carries a contract, the harness honours it

Every captured case now carries `expected.body_comparison`:

| Contract | Meaning | Cases |
|---|---|---|
| `exact` | The live body must equal the recorded body **byte for byte**. The corpus marks the case deterministic, so the bytes are the contract. | 50 |
| `structural` | A STEP export: the corpus records the stable structural fields (an ISO 10303-21 file with one manifold solid) instead of the bytes. | 2 |
| `fields` | The body carries values no run can reproduce. The corpus declares **every one of them by path**, with the JSON types and the shape the live value must have. | 4 |

The `fields` cases carry `expected.volatile_paths`, for example:

```json
{"path": "$.events[*].event_digest", "types": ["string"], "format": "sha256-hex",
 "reason": "the chain digest binds the STEP artifact digest, so it is not reproducible"}
```

The declarations come from three places, all of them the corpus's own
metadata rather than a guess:

* the fields the capture already masked (`volatile_fields`, with their
  reasons);
* the leaves the capture masked **by value** — a digest that appears in more
  than one place, whose path the capture could not name. The generator finds
  them and names them (`$.events[3].payload.step_sha256`,
  `$.events[5].payload.sha256`, `$.events[6].payload.step_sha256`);
* the paths this generator adds because a running service reports a
  different value than the fixture clock does (`$.events[*].occurred_at`),
  with the reason the corpus's own `nondeterministic_fields` already gives.

The harness masks the declared paths on **both** sides and compares the rest
of the document exactly. It also checks that the live value at a declared
path still has the declared JSON type and shape, so a regression cannot hide
behind a mask: a digest must still be 64 lowercase hex characters, a
timestamp must still be an ISO-8601 UTC timestamp with the recorded
precision, and the CAD probe's `available` must still be a boolean.

### 3.2 The host-evidence case is treated explicitly, both ways

`studio.cad_providers` is now declared as host evidence, in the corpus and in
the harness:

* **In the corpus** (`backend/compat/generate_corpus.py`): the case is marked
  `host_dependent: true` with a recorded rationale, a `host_dependence`
  block (the rationale, the probe, the dependent fields), and the three
  host-dependent values (`available`, `version`, `reason`) are masked with
  `<VOLATILE>`. The recorded body keeps the host-independent part — the
  provider set, the order, the display names, the modes, the measurement
  source — and the `content-length` header is declared host-dependent
  (`host_dependent_headers`) instead of being frozen at a number that only
  this generation host produced.
* **In the harness**: the case is compared against the **current** host
  probe. The harness rebuilds the *worker's own* child environment with the
  product's `build_child_environment` helper (the same function the Rust
  front door ports), runs the product's own `probe_providers()` in it, and
  compares the answer's `id`, `name`, `available`, `version`, `reason`,
  `measured_by` and `mode` with that probe. The comparison is therefore
  against the probe of *this* host, not against the probe of the machine that
  generated the corpus.

Both options the task offered are used, because they answer different
questions: the corpus's marker says *which* values are host evidence, and
the current-host probe says *what this host* currently reports.

The consequence is that the **corpus bytes no longer depend on the host**:
the corpus is now reproducible on a host with a different FreeCAD
installation, and `generate_corpus.py --check` no longer breaks when the
probe changes. The generation-time probe is still recorded — as an
environment fact in `manifest.oracle_environment.cad_providers` — and it is
now declared in `manifest.determinism.excluded_from_comparison` with its
reason, because it is a fact about the host and not about the product.

### 3.3 What was *not* loosened

* No case was skipped. The corpus still has 56 cases and the harness still
  replays all of them.
* The two `body_deterministic: false` cases that are **not** host probes
  (the design result and the two journal cases) are compared *more* strictly
  than before, not less: the whole document outside the declared paths must
  match exactly, including the measured numbers, the check rows, the mesh and
  the event payloads.
* The deterministic cases are untouched: 50 of them are still compared byte
  for byte, and the negative controls below prove the byte comparison still
  bites.
* The two `oversized_*` cases remain the only ones the harness does not
  replay over a socket, for the reason that has always been recorded in the
  harness itself (the CLI does not expose the corpus harness's 16,384-byte
  body limit); they are covered by the library-level `corpus_fixture` test.
  This is unchanged from the previous version of the harness and is *not* a
  new skip.

### 3.4 The corpus metadata that changed shape

`body_length` now means *the length of `body_text`*, and
`body_length_role` says so on every case that has a body text. Before this
cell, `body_length` was the raw response length while `body_sha256` was the
digest of the **masked** text — two different byte strings. The three
masked cases are affected (`4360 → 4252`, `8087 → 6305`, `135 → 81`), and
the CAD case is new (`783 → 487`, the masked length). The raw length is still
frozen where it is stable, in the recorded `content-length` header, which the
harness compares for every case except the one that declares it
host-dependent.

---

## 4. `corpus_fixture.rs`: the regenerated manifest

The test no longer pins the stale 48-case corpus. It now asserts the
regenerated manifest:

* `manifest.corpus_sha256 == "53dcb9da105589b5a92d37b16d63c7fe2d5e60b7630549102127e019c95dff28"`;
* the digest **recomputed from the corpus files on disk** with the
  generator's own rule (the SQLite file's entry is `not-asserted`), so a
  corpus that is not the one the manifest describes fails the test;
* 56 cases, 15 declared Studio routes, no uncovered route;
* every case declares a known comparison contract, and all three contracts
  occur.

The replay itself now covers all 56 cases. The eight cases whose route the
front door proxies to the supervised geometry worker (ADR-0010) are
recognised by their route, not by a second list of case ids, and are
asserted to produce the truthful refusal of the offline build: a 501
`geometry_route_unavailable` that names the internal worker route the front
door would have called. Their contract against the *live* worker is the live
worker differential's (292 cases) and the corpus replay harness's, which both
run against the real worker.

One more correction was needed and is worth recording: the test used to
normalise only the **answer** and compared it with the **raw** recorded
text. That worked while the corpus predated ADR-0009; the regenerated corpus
carries `design_available` and `design`, so both sides are normalised now.

---

## 5. Verification

Every command below was run from the repository root (or from
`backend/crates/kala-studio`) with the repository virtual environment and
with the data isolation of the conventions in force (see §6).

### 5.1 The corpus replay

```
$ .venv/bin/python backend/crates/kala-studio/tests/replay_binary_corpus.py
release binary cases: 56
exact: 48
fields (the corpus declares the volatile paths): 4
  of which host evidence (compared against the current host probe): 1
step structural (real ISO 10303-21 solid): 2
skipped for library body-limit config: 2
  skip studio.negative.oversized_declared: requires 16,384-byte library config; covered by corpus_fixture
  skip studio.negative.oversized_chunked: requires 16,384-byte library config; covered by corpus_fixture
corpus digest: 53dcb9da105589b5a92d37b16d63c7fe2d5e60b7630549102127e019c95dff28
mismatches: 0
```

Exit code 0.

### 5.2 The negative controls — a mutated corpus must be refused

The harness gained `--fixture`, so a mutated **temporary copy** of the
corpus can be replayed. The checked-in corpus is never touched. Five
mutations, one byte each, in five different kinds of case:

| Mutated case | Kind | Reported | Exit |
|---|---|---|---|
| `studio.health` | exact, ADR-normalised | `studio.health: document mismatch` | 1 |
| `studio.preview.box_default` | exact, plain | `studio.preview.box_default: body mismatch` | 1 |
| `studio.design.l_bracket` | fields | `the answer differs from the corpus outside the declared volatile fields` | 1 |
| `studio.journal.page` | fields | `the answer differs from the corpus outside the declared volatile fields` | 1 |
| `studio.cad_providers` | fields + host evidence | `the answer differs from the corpus outside the declared volatile fields` | 1 |

The last one is the important one: it mutates a **host-stable** field of the
host-evidence case (`providers[2].id`), so the mask does not hide it.

The current-host-probe comparison was checked separately, so that it is not
merely *counted*: the probe helper returns this host's real probe
(`opencascade 8.0.1.1`, `freecad 1.1.4`, `solidedge` refused), and a
deliberately wrong answer is reported field by field —
`opencascade.available is False, the current host probe reports True`,
`opencascade.version is None, the current host probe reports '8.0.1.1'`,
`the answer does not name freecad`, and so on.

### 5.3 The corpus checks

| Command | Result |
|---|---|
| `.venv/bin/python backend/compat/generate_corpus.py` | exit 0; 56 Studio cases, 15 routes, 0 uncovered; `studio_body_comparisons = {exact: 50, fields: 4, structural: 2}` |
| `.venv/bin/python backend/compat/generate_corpus.py --check` | exit 0 — `determinism check passed` |
| `.venv/bin/python backend/compat/self_check.py` | exit 0 — `self-check OK`, `corpus_sha256 53dcb9da…`, `studio_http_cases 56`, `studio_routes 15` |
| two generations into two directories | byte-identical in all 13 files (the corpus digest `53dcb9da…` was produced twice from two independent runs) |
| `.venv/bin/python -m ruff check backend/compat` | all checks passed |
| `.venv/bin/python -m ruff check backend/crates/kala-studio/tests/replay_binary_corpus.py` | all checks passed |

The self-check's own summary:

```json
{
  "canonical_json_cases": 66, "chain_events": 5, "cli_commands": 12,
  "cli_executions": 9, "core_http_cases": 49, "core_routes": 17,
  "corpus_sha256": "53dcb9da105589b5a92d37b16d63c7fe2d5e60b7630549102127e019c95dff28",
  "decimal_cases": 85, "domain_packages": 3, "geometry_meshes": 6,
  "regenerated": true, "studio_http_cases": 56, "studio_routes": 15
}
```

### 5.4 The Rust suite

| Command | Result |
|---|---|
| `cargo test --locked --release --all-features --no-fail-fast` (kala-studio) | **166 passed, 0 failed, 0 ignored** |
| `cargo test --locked --release --no-fail-fast` (kala-studio, default features) | **149 passed, 0 failed, 0 ignored** |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets --all-features -- -D warnings` | clean |
| `cargo build --release` | ok |

The count moved from the 165 the `docs-final` cell measured (164 passed,
1 failed) because this cell adds one test — the manifest assertion — and
because the failing corpus fixture now passes.

### 5.5 The differentials

| Command | Result |
|---|---|
| `.venv/bin/python backend/crates/kala-studio/tests/python_differential.py` | **268 cases, 0 mismatches** |
| `.venv/bin/python backend/crates/kala-studio/tests/worker_differential.py` | **292 cases, 0 mismatches, 0 known gaps** |
| `.venv/bin/python backend/crates/kala-studio/tests/python_provider_dto_oracle.py` | **20 cases, 0 failures** |

None of the three reads the corpus, so they are unaffected by the
regeneration; they are reported because the task asks for them to stay green.

---

## 6. The USER DATA rule

Every shell session of this cell began with:

```bash
export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo
export HOME=$(mktemp -d) KALA_AGENT_DATA_DIR=$(mktemp -d) \
       KALA_STUDIO_DATA_DIR=$(mktemp -d) XDG_DATA_HOME=$(mktemp -d)
```

and every binary, Python entry point and differential was given an explicit
temporary `--data-dir` (the replay harness: `--data-dir <tmp>/data`; the
corpus generator: a `tempfile.TemporaryDirectory` work root; the Rust tests:
`tempfile::tempdir()`).

The harness's own host probe runs with the environment the front door gives
its geometry worker, whose home is `<tmp>/data/geometry-worker` — a
temporary directory, never the user's home. **No path under
`/home/v9i/.local/share/kala-agent` was read, written, listed or named in any
command of this cell.** No credential was read; the only tokens in play are
the corpus's synthetic fixtures.

No commit, push, deployment or delegation was performed.

---

## 7. Compatibility and safety notes

* **No product behaviour changed.** The contract, the mask and the probe are
  fixture-side. The corpus still records what the product produces.
* **The comparison is not weaker.** The negative controls of §5.2 show that
  a single mutated byte in a deterministic case, in a `fields` case and in a
  host-evidence case is refused.
* **The host probe is compared against the current host**, in the worker's
  own environment, so a host change is reported as a mismatch rather than
  hidden.
* **The corpus is host-independent for the probe.** The recorded bytes of
  `studio.cad_providers` no longer contain the probe result, so the corpus
  regenerates identically on a host with a different FreeCAD installation.
  The generation-time probe stays recorded as an environment fact in
  `manifest.oracle_environment.cad_providers`, declared as excluded from the
  comparison with its reason.
* **The masked values are never written down.** `<VOLATILE>` replaces the
  STEP digest and the chain digests, as before; the new mask replaces the
  CAD probe result. `self_check.py` verifies that every masked case still
  carries its placeholder, its reason and a matching digest.

---

## 8. Remaining gaps and handoff

1. **Two READMEs carry the old corpus digest and are not this cell's
   files.** `backend/compat/README.md` line 44 and
   `backend/fixtures/README.md` lines 38 and 166 still print
   `9993939bfea5e01553438e680fe3a9ea4b24335ee9077da5b8dc453b5a987e8b`.
   The digest is now
   `53dcb9da105589b5a92d37b16d63c7fe2d5e60b7630549102127e019c95dff28`.
   `backend/compat/README.md` belongs to the `compat-drift` cell and
   `backend/fixtures/README.md` to the `corpus`/docs cells; this cell edited
   neither.
2. **The corpus is not committed.** The whole `backend/` tree is untracked
   (`git status --porcelain` shows `?? backend/`), so the regenerated corpus
   and its digest live only in the working tree. No commit was made, as the
   conventions require.
3. **The six new routes are still not covered by the 268-case live
   differential.** They are covered by the 292-case worker differential and
   by the corpus replay harness (which now runs them against the real
   worker). The native-vs-Python byte parity of those routes is the delivery
   cell's open item.
4. **The Rust `corpus_fixture` test cannot serve the worker-proxied
   routes.** It attaches no geometry worker, so those eight cases are
   asserted to produce the truthful 501. A test that attached the real
   worker would be a Python-level test, not a Rust unit test.
5. **The `manifest.oracle_environment.cad_providers` entry is now excluded
   from the comparison.** It is still recorded. A reader who wants the
   determinism check to fail when the host's FreeCAD installation changes
   can re-include it, at the price of a host-dependent check.
6. **Everything the previous cells listed as a gap still stands** — Windows /
   COM / NSIS, the 17-vs-18 core route question, the STEP 503 branch, the
   live-socket capture, `KALA_STUDIO_MESH_LIMIT`, and the timing deferral.
   This cell changed none of them.
7. **No runtime cutover, packaging decision, Windows runtime, Solid Edge
   build, speed or readiness claim is made in this report.**

---

## 9. The short version

The corpus was not wrong about the product; it was silent about how its own
bodies may be compared, and the harness guessed. The corpus now declares a
comparison contract per case, names every value it cannot reproduce, and
marks the one case that is a live probe of the host as host evidence with a
recorded rationale; the harness honours all three, and additionally checks
the host-probe case against the probe of the current host. The regenerated
corpus carries 56 cases, 15 routes and the digest
`53dcb9da105589b5a92d37b16d63c7fe2d5e60b7630549102127e019c95dff28`; the
replay is 56 cases and 0 mismatches; five one-byte negative controls are
refused; the `kala-studio` suite is green (166 with `--all-features`, 149
without); the corpus self-check and determinism check are green; and the
268 / 292 / 20 differentials are unchanged at 0 mismatches.
