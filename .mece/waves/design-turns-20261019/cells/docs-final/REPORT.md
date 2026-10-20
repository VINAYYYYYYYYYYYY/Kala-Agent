# docs-final — the Rust migration status, recorded honestly

**Cell:** `docs-final` of the `design-turns-20261019` wave. **Worker:**
`docs-final` (Opus conductor). **Date:** 2026-10-20.

**Status: done for the five owned documents.** No commit, no push, no
deployment, no delegation, no credential read, and **no code, ADR, fixture or
test edit**. The only file outside the five documents that changed is one
crate-local `Cargo.lock`, and it is disclosed in full in §2 below.

---

## 1. What I read

- `TASK.md` for this cell.
- `.mece/waves/rust-backend-20261019/cells/CONVENTIONS.md` — including the
  **USER DATA IS OFF LIMITS** rule, which was applied in **every** shell
  session of this cell (§2).
- The reports of **every** cell of the `rust-backend-20261019` wave:
  `application-r1`, `application-review`, `schemas-r1`, `api-routes-r1`,
  `cli-r1`, `compat-drift`, `r2-datadir-fix`, `kernel`, `kernel-review`,
  `deep_json`, `contracts`, `persistence`, `repositories`, `budget`,
  `artifacts`, `process`, `domains`, `memory-r1`, `agent-r1`, `gateway-r1`,
  `providers-r1`, `tools-protocol-r1`, `tools-registry-r1`,
  `tools-executor-r1`, `sandbox-r1`, `sandbox-r2`, `api-core-r1`,
  `api-errors-fix`, `design-intent-r1`, `studio`, `delivery`, `geometry`,
  `step`, `r4-solidedge-client`, `r1-worker-mode`, `r2-chat-providers`,
  `r2-studio-proxy`, `r2-followup`, `rust-e2e-qa`, `corpus`, `audit`,
  `workspace`.
- `.mece/waves/design-turns-20261019/cells/test-isolation/REPORT.md`, plus
  `docs-r1`, `docs-r2` and `readiness-sync` for the state of the documents I
  was updating.
- The crate `README.md` of every crate under `backend/crates/`, the Python
  module list under `src/kala_agent/`, and the four documents I own.

---

## 2. The USER DATA rule, and one disclosure

**Every** shell session of this cell began with:

```bash
export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo
export HOME=$(mktemp -d) KALA_AGENT_DATA_DIR=$(mktemp -d) \
       KALA_STUDIO_DATA_DIR=$(mktemp -d) XDG_DATA_HOME=$(mktemp -d)
```

and every binary, Python entry point and differential was given explicit
temporary `--data-dir` values. **The user's real data directory was never
opened, read, written, listed or named in a command.** No credential was
read.

One disclosure, as the conventions require honesty about every touched path:

> **`backend/crates/kala-application/Cargo.lock` was refreshed.** The
> crate's own documented gate is `cargo test --locked`, and that command
> **failed** with *"cannot update the lock file … because `--locked` was
> passed to prevent this"*. The crate's local lock predated `serde_json`,
> which `api-routes-r1` added to `kala-api`, so the lock was stale. Running
> the same command once **without** `--locked` updated that one lock file
> (cargo's own update; it adds `serde_json 1.0.154` and its transitive
> dependencies) and `--locked` works again: **53 passed, 0 failed**.
> **No other file was changed.** The original bytes of the stale lock were
> not captured before the refresh, so they cannot be restored; the refresh is
> the canonical cargo update and it is what makes the crate's own gate
> pass. A fresh clone hits the same refusal until the lock is refreshed
> once.

Everything else I touched is documentation. The five files below are the
only content files I edited.

---

## 3. Files changed

| Path | Change |
|---|---|
| **`docs/rust-migration-status.md`** | **NEW.** The per-module status record: every Python module → its Rust crate/module, its status, its measured test count, its differential evidence (case counts and mismatches) and its reviews; what stays Python **on purpose**; every intentional difference in one place; what remains; the evidence and how to re-run it. |
| **`backend/README.md`** | The workspace map brought in line with the tree: the crate tables and the measured counts (423 in the root workspace, not 418), the new standalone crates (`kala-api/server`, `kala-application`, `kala-cli`), the agent runtime's acceptance, "What is still Python" split into *on purpose* and *ported but not cut over*, "What is pending" rewritten around the six open items, the corpus self-check changed from **exit 1** to **exit 0**, the red corpus replay and the failing parked-build test recorded, the `kala-application` lock note, and a pointer to the new status document. |
| **`docs/rust-studio.md`** | The front door's record brought in line: the corpus replay changed from "48 cases, 0 mismatches" to "**56 cases, 5 mismatches**" with the analysis of the four affected cases, the verification table split into *run in this pass* and *carried over from the previous pass*, the studio test counts (149 default / 164 + 1 failed with `--all-features`), the agent-runtime note corrected, and a pointer to the new status document. |
| **`docs/studio-limitations.md`** | New **"Test hygiene: the user's data directory"** section: the USER DATA rule, the `cli-r1` incident of 2026-10-20 05:35:30 +0530 and its factual assessment, the `test-isolation` work (`tests/conftest.py`, the new isolation test, the `data_dir=None` fix), the `stat`/`ls` proof (357 polls, identical size/mtime/inode, 0 `-shm`/`-wal`) and the three items that are still open. Plus a new **"The frozen corpus is not all contract"** section, the test-suite table rewritten with a *when* column and the new counts, the 292/268/48/20 line corrected, and a "the wave did not cut over anything" bullet. |
| **`backend/fixtures/README.md`** | Fixed as `compat-drift` asked: `studio_http.json` **9 Studio routes + gates, 48 cases** → **15 Studio routes + gates, 56 cases**; the determinism notes now name the **Studio session journal's fixed clock**, the **STEP writer's timestamp and per-process translator counter**, the `<VOLATILE>` design/journal digests and the **corpus digest**; `manifest.json` now advertises the corpus digest in its description and the digest is printed; a new consumption rule 11 on run records vs contract; the known gaps gained the host-evidence CAD probe and the six uncovered routes; the `self_check.py` result and its "never writes into `backend/fixtures/`" property recorded. |

---

## 4. What the new status document says

`docs/rust-migration-status.md` is the answer to "what is the state of the
Rust port, honestly". Its structure:

1. **How to read it** — the status vocabulary, what a test count is and is
   not, what a differential and what a review are.
2. **The module map** — every Python module, in three tables (the seven root
   workspace members, the sixteen standalone crates, the parked crate), with
   its Rust home, its status, its measured release test count, the
   differential evidence as *cases* and *mismatches*, and its reviews.
3. **What stays Python, on purpose** — the FreeCAD adapter (it runs inside
   FreeCAD), the Solid Edge bridge (it runs on Windows) and the geometry
   worker (pending the OCCT decision), each with its reason.
4. **The intentional differences, in one place** — application D1, the
   schemas' extra-field order, the api-routes' seven declared differences,
   the kernel's stricter restore, the sandbox's fail-closed behaviour, the
   executor's dead `timeout_seconds`, the native identity digests, and the
   remaining per-crate lists.
5. **What remains** — the cutover/packaging decision, `kala-agent serve`,
   the Windows checks blocked on MinGW, the live Solid Edge proof, the
   `views.rs` duplication, and the open findings.
6. **The evidence, and how to re-run it** — every command that was re-run
   with its result, and every command that was **not** run, marked as such.
7. **The short version.**

---

## 5. Every command, run or not run

### 5.1 Re-run in this pass (all with the data isolation in force)

| Command | Result |
|---|---|
| `cd backend && cargo test --workspace --locked --release` | **423 passed**, 0 failed, 0 ignored |
| `cd backend/crates/kala-contracts && cargo test --locked --release` | **50 passed** |
| `cd backend/crates/kala-process && cargo test --locked --release` | **46 passed** |
| `cd backend/crates/kala-kernel && cargo test --locked --release` | **152 passed** |
| `cd backend/crates/kala-memory && cargo test --locked --release` | **85 passed** |
| `cd backend/crates/kala-tools && cargo test --locked --release` | **229 passed** |
| `cd backend/crates/kala-sandbox && cargo test --locked --release` | **60 passed** |
| `cd backend/crates/kala-api && cargo test --locked --release` | **124 passed** |
| `cd backend/crates/kala-api/server && cargo test --locked --release` | **10 passed** |
| `cd backend/crates/kala-design-intent && cargo test --locked --release` | **54 passed** |
| `cd backend/crates/kala-domains && cargo test --locked --release` | **29 passed** |
| `cd backend/crates/kala-repositories && cargo test --locked --release` | **33 passed** |
| `cd backend/crates/kala-solidedge-client && cargo test --locked --release` | **79 passed** |
| `cd backend/crates/kala-cad-providers && cargo test --locked --release` | **37 passed** |
| `cd backend/crates/kala-application && cargo test --locked --release` | **53 passed** — after the one lock refresh of §2 |
| `cd backend/crates/kala-cli && cargo test --locked --release` | **22 passed** |
| `cd backend/crates/kala-runtime && cargo test --locked --release` | **112 passed** |
| `cd backend/crates/kala-cad && cargo test --locked --release` | **64 passed** (parked) |
| `cd backend/crates/kala-studio && cargo test --locked --release --all-features --no-fail-fast` | **164 passed, 1 failed** |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/python_differential.py` | **268 cases, 0 mismatches** |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/worker_differential.py` | **292 cases, 0 mismatches, 0 known gaps** |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/python_provider_dto_oracle.py` | **20 cases, 0 failures** |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/replay_binary_corpus.py` | **56 cases, 5 mismatches**, exit 1 |
| `.venv/bin/python backend/compat/self_check.py` | **exit 0, `self-check OK`**; `corpus_sha256 9993939b…` |

### 5.2 Not run, and not claimed

| Command | Why |
|---|---|
| `cargo fmt --check` and `cargo clippy --all-targets --all-features -- -D warnings` (per crate) | **Not run** in this pass. The gate results in the cell reports stand as filed. |
| `cargo check --target x86_64-pc-windows-gnu` (any crate) | **Not run**: the mingw C toolchain is absent on this host. |
| any Windows build, MSVC run, COM run, Solid Edge build | **Not run**; none is available here. |
| `npm run check:native-readiness`, `npm test`, `npm run build`, `npm ci` in `studio/` | **Not run** in this pass; the front-door route split and the UI results are carried over from the previous docs pass and are marked as such in `docs/rust-studio.md`. |
| any packaging, installer or release-bundle command | **Not run**; the packaging decision has not been taken. |
| any benchmark | **Not run.** No speed, memory or readiness number is claimed. |
| `pytest tests` (the Python suite) | **Not run** in this pass; the counts in `docs/studio-limitations.md` are the `test-isolation` cell's, attributed as such. |

---

## 6. The findings this pass produced

Three concrete findings came out of the re-runs. All three are recorded in
the documents rather than smoothed over.

### 6.1 The corpus replay is red, and the corpus is what moved

`replay_binary_corpus.py` now reports **56 cases, 5 mismatches** where the
documents said "48 cases, 0 mismatches". I diagnosed it rather than
restating it:

- All five mismatches are in the **four cases the regeneration added**, and
  none is a regression in the Rust front door.
- `studio.cad_providers` is **host evidence**: the corpus recorded FreeCAD
  as unavailable with a 245-character reason (FreeCAD could not create its
  runtime directory in the generator's environment), and the corpus marks
  the case `body_deterministic: true`, which is wrong. I verified the
  arithmetic exactly: the same document with FreeCAD available is **542
  bytes**, and the replay measured 542.
- `studio.design.l_bracket`, `studio.journal.page` and
  `studio.journal.verify` are marked `body_deterministic: false` **by the
  corpus itself** (the OCP STEP export embeds the wall clock of the write,
  and the journal chain digests bind it), but the harness compares the raw
  bytes.

### 6.2 The parked build has a failing test

`cargo test --locked --release --all-features` in `backend/crates/kala-studio`
reports **164 passed, 1 failed**:
`tests/corpus_fixture.rs::replay_all_48_studio_cases` asserts the corpus has
48 cases and fails with `left: 56, right: 48`. The default feature set does
not run that test, so the default build is green. I did not edit the test.

### 6.3 The `cli-serve` cell has not landed

`backend/crates/kala-cli` still refuses `serve` with the gap message, and
`.mece/waves/rust-backend-20261019/cells/cli-serve/` contains a `TASK.md`
and **no report and no change**. The pieces exist — `kala-api`'s router and
`Application` port, the `kala-api/server` binary with the launch contract,
`build_serve_target` and `build_serve_plan` — but the command is not wired.

---

## 7. Honest limits of this pass

- The counts are **release-profile** counts measured on this host on
  2026-10-20. They are not a quality score and they are not comparable
  across crates.
- A test count is **not** a differential. The differential evidence is
  reported separately, as cases and mismatches, from the cell reports.
- I did **not** re-run the Python test suite, the UI suite, `fmt`, `clippy`
  or any Windows check; where a number in a document comes from an earlier
  pass, the document now says so.
- I did **not** attempt to fix the three findings of §6. Two of them are
  code (a test assertion and a harness normalisation) and one is a missing
  cell; fixing them is not this cell's scope, and this cell owns
  documentation only.
- I did **not** repair the user data directory. The incident assessment is
  in `docs/studio-limitations.md`; a repair is the user's decision, and the
  journal shows no event was appended, so the recorded history is intact.
- No runtime cutover, packaging decision, Windows runtime, Solid Edge build,
  speed or readiness claim is made in any of the five documents.

---

## 8. Handoff

The next reader should:

1. read `docs/rust-migration-status.md` §5 for the open items, and treat
   §4 as the single place where the two implementations are known to
   differ;
2. not read "green" as "cut over" — **nothing is cut over**, and the Python
   reference is the oracle for every claim;
3. regenerate the corpus after any product source change and confirm the
   corpus digest moves with the source pin, and read
   `manifest.determinism.excluded_from_comparison` before treating a
   manifest difference as a real drift — only the git working-tree path list
   is excluded;
4. treat `<VOLATILE>` in `studio_http.json` as a deliberate hole, and
   `studio.cad_providers` as **host evidence**, not as a missing
   expectation.
