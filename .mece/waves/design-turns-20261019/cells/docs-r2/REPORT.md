# docs-r2 — docs refreshed for the Rust front door, the gateway and the executor

Cell: `docs-r2`, wave `design-turns-20261019`. Date: 2026-10-20.

Read before starting: this cell's `TASK.md`, `../docs-r1/REPORT.md`, and
the REPORTs of `.mece/waves/rust-backend-20261019/cells/`
`r2-chat-providers`, `gateway-r1`, `tools-executor-r1`,
`tools-registry-r1` and `rust-e2e-qa`. I also read the current
`kala-studio` crate README and route source, the readiness script, and the
current docs I own.

Status: **done** for the four owned docs. No commit, push, deploy,
delegation or credential read was performed. **No code, test, fixture or
ADR was edited.** A throwaway `KALA_STUDIO_TOKEN` was supplied in the
launch environment for the live curls; it is not a stored credential and
was not read from any file.

---

## 1. Files changed

| File | Change |
|---|---|
| **`docs/cad-providers.md`** | Added **“Chat and design through either front door”**: the selected provider is passed to the worker unchanged, FreeCAD builds through Rust chat with the live numbers, Solid Edge refuses honestly on Linux with `design: null`, no fallback is tried, and no provider means nothing is built. Added the cold-reload area note to the journal section. Removed the now-false limitation that the Rust front door cannot build with FreeCAD or Solid Edge; replaced it with the real remaining limits (Solid Edge unproven, front door is a front door only). Updated the FreeCAD test observation to the re-run `8 passed in 19.96 s`. |
| **`docs/studio-limitations.md`** | Replaced the “front door cannot build” CAD bullet with the accepted behaviour. Added the provider pass-through and the 292/268/48/20 live differential to the Rust-front-door section. Added **“The cold-reload surface-area bug (fixed)”** with the reader-only fix, the old-reader failure, the tests and the live cold-restart check. Updated the test-suite table to the observed counts (1555 unit, 189 security, 8 FreeCAD, 619 UI, 418 workspace, 229 tools, gateway 55 accepted / 54 re-run functions, 292 differential, READY 6/10). Marked the manual smoke, install and adversarial steps that were not re-run. Added the in-progress agent runtime to “what this wave did not test”. |
| **`backend/README.md`** | `kala-studio` is now 148 default / 164 all-features; the root workspace is **418 tests**. `kala-tools` is **229** and includes the registry, simulator and executor. `kala-runtime` is **accepted as the gateway (55 tests)** with `agent-r1` **in progress**; the old “not accepted, 1 of 2 failed” note is gone and replaced by the current, narrower caveat. Added the standalone live differentials (26 mock-provider cases, 3,324-entry corpus, 23 executor cases, 1,064 registry/simulator comparisons). The worker differential is now **292**. Updated “what is still Python”, “what is pending”, build/test counts, the gateway rerun command and the standalone-crate note. Recorded that `backend/compat/self_check.py` exited 1. |
| **`docs/rust-studio.md`** | **Rewritten** from the stale nine-route description to the current front door: three layers; the **6 native / 10 proxied** route table (15 Python route-methods, 16 front-door route-methods, 4 worker-only internal routes); worker command/token/lifeline/readiness/restart/503 behaviour; chat and design provider pass-through with FreeCAD and Solid Edge evidence; the shared `KALA_STEP_IMPORT_MAX_BYTES` ceiling; the live differentials (292/268/48/20); build/run `curl` commands; native staging and the current READY-but-packaging-blocked state; an observed-results table and an explicit not-run list. |

Files I did **not** touch: every ADR, every other doc (including
`docs/studio.md`, `docs/studio-install.md`, `docs/studio-providers.md`,
`solidedge.md`, `how-to-read.md`, `student-guide.md`, `threat-model.md`,
`verification-report.md`, `architecture-report.md`, `review1.md`), the root
`README.md`, `studio/README.md`, and all code, tests, fixtures, manifests
and locks.

`docs/studio-limitations.md` was already modified in the working tree by
`docs-r1`; the diff shown by git therefore contains both that prior work and
my incremental changes. I did not revert or rewrite the prior work.

---

## 2. Commands run (and results)

All commands below were run on this host on 2026-10-20: Linux,
`.venv` CPython 3.14.8, OCP 8.0.1.1, FreeCAD 1.1.4, rustc/cargo 1.99.1,
Node 26.8.2, npm 11.19.1.

### Rust and UI

| Command | Result |
|---|---|
| `cd backend && cargo test --workspace --locked --release` | **418 test functions, 0 failed, 0 ignored** (`--list` count; the log prints 422 “passed” lines because the artifact concurrency test runs nested child processes) |
| `cd backend && cargo test --workspace --locked` (debug) | **418 test functions**, 0 failed on the final full re-run. The first debug run had one flaky failure in `the_chat_turn_is_journalled_through_the_worker` (journal 3 events vs 1); the same test passed alone and the full debug re-run passed. |
| `cd backend && cargo test -p kala-studio --locked --release --all-features` | **164 passed**, 0 failed |
| `cd backend && cargo fmt --all --check` | clean, exit 0 |
| `cd backend && cargo clippy --workspace --all-targets --locked -- -D warnings` | clean, exit 0 |
| `cd backend && cargo build --workspace --release --locked` | success |
| `cd backend && cargo build --release --locked -p kala-studio` | success; release binary `backend/target/release/kala-studio`, SHA-256 `55a3f94d18ab0534473fd12ff02418bc2457c05b78dc27f14aa4e4b8ea83e0e8`, 7,254,552 bytes |
| `cd backend/crates/kala-tools && cargo test --locked --release` | **229 passed**, 0 failed; 17 test binaries including the executor suites |
| `cd backend/crates/kala-kernel && cargo test --locked --release` | **152 passed**, 0 failed |
| gateway targets in `kala-runtime`: `cargo test --locked --release --lib --test gateway_semantics --test gateway_security --test python_differential --test corpus_differential` | **54 passed** (37 semantics, 14 security, 2 mock-provider differential, 1 corpus). The accepted gateway total is **55** with its library doc-test. |
| `cd studio && npm run typecheck` | clean, exit 0 |
| `cd studio && npm test` | **43 test files, 619 tests passed** |
| `cd studio && npm run build` | green, `✓ built in 581ms`; only the pre-existing `three` >500 kB chunk warning |
| `cd studio && npm run check:native-readiness` | **READY**: 15 Python route-methods covered, **6 Rust-native**, **10 proxied**, 0 missing; 4 worker-only internal route-methods |
| `cd backend && cargo check --locked --offline --target x86_64-pc-windows-msvc --all-targets -p kala-json -p kala-geometry -p kala-budget -p kala-step -p kala-artifacts` | **exit 0** (the five original pure-Rust libraries, compile check only) |
| the matching `cargo clippy ... -- -D warnings` for the same five crates and target | **exit 0**, clean |

### Python and live differentials

| Command | Result |
|---|---|
| `.venv/bin/python -m pytest tests/unit -q` | **1555 passed**, 0 failed, 18 warnings |
| `.venv/bin/python -m pytest tests/security -q` | **189 passed**, 0 failed, 1 warning |
| `.venv/bin/python -m pytest tests/unit/studio_design/test_cad_providers_freecad.py -q` | **8 passed in 19.96 s** (real `FreeCADCmd`) |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/worker_differential.py` | **292 cases, 0 mismatches, 0 known gaps** |
| `.../python_differential.py` | **268 cases, 0 mismatches** |
| `.../replay_binary_corpus.py` | **48 cases, 0 mismatches** |
| `.../python_provider_dto_oracle.py` | **20 cases, 0 failures** |
| `.venv/bin/python backend/compat/self_check.py` | **exit 1** — fresh regeneration differs from the checked-in corpus for `cli_launch.json`, `manifest.json` and `studio_http.json`. Recorded in `backend/README.md`; no corpus file was regenerated. |

### Live front-door run

The exact launch block now in `docs/rust-studio.md` was executed with
`--port 8796`, `--data-dir /tmp/kala-studio-data` and a throwaway
`KALA_STUDIO_TOKEN`. The front door supervised its own worker. Observed:

| Probe | Result |
|---|---|
| `GET /api/studio/health` | 200; `geometry_worker {state: "ready", version: "0.1.0", design_available: true}`; `design_available: true` |
| `GET /api/studio/providers/cad` | OpenCascade 8.0.1.1 available, FreeCAD 1.1.4 available, Solid Edge unavailable with the Windows-bridge/Linux reason |
| `POST /api/studio/chat` with `cad_provider: "freecad"` | 200; design `a9d04f8ff14e365ee57e0a7d6e212ad5afae59de428b5cefa029bf91f72f62a7`; provider `freecad`, version `1.1.4`, `measured_by: "FreeCAD"`; volume **35571.416047 mm³**, area **11889.101731 mm²**, envelope 60 × 45 × 40 mm, `verified: true` (5/5), view 568 triangles / 586 vertices / 1 solid |
| `POST /api/studio/chat` with `cad_provider: "solidedge"` | 200; the worker's exact refusal (*“Solid Edge build requires the Windows bridge (not reachable); this host is linux, so the ADR-0008 bridge cannot run here. Available: opencascade, freecad. No design was built.”*), `design: null`, no fallback |
| `POST /api/studio/chat` with no `cad_provider` | 200; provider-choice prompt, `design: null` |
| `GET /api/studio/journal/verify` | 200 on the earlier live run; `{"ok": true, "event_count": 15, "head_digest": "...", "first_bad_sequence": null}` |
| Cold reload (earlier data directory) | front door stopped and restarted against the same data directory; the FreeCAD design came back `cached: true` with `surface_area_mm2: 11889.101731` |

The refusal, no-provider, journal and cold-reload probes were run on an
earlier data directory in this pass; the exact FreeCAD and health/provider
block was re-run against `/tmp/kala-studio-data` after the docs were
written. All running servers were stopped afterwards.

### Not run (marked in the docs where they appear)

| Command / action | Why |
|---|---|
| `pip install -e ".[studio]"` / `".[geometry]"` | Network install; the venv already has the dependencies. Marked “not run” in the limitations test procedure. |
| `npm ci` | Dependencies are already installed; the test/typecheck/build gates ran against the installed tree. Marked “not run”. |
| `python packaging/studio_smoke.py --backend dist/kala-studio` | No frozen backend was built in this docs pass; marked “not run”. |
| `npm run build:native` | Staging was not rebuilt; the doc describes the helper from its source and marks the command not run. |
| `npm run desktop`, `npm run desktop:pack -- --dir` | Need a desktop session and the packaging toolchain; release packaging is blocked anyway. |
| Windows studio cross-check | `cargo check -p kala-studio --target x86_64-pc-windows-gnu` is blocked by the missing MinGW C toolchain (`ring` needs `x86_64-w64-mingw32-gcc`). The five pure-Rust libraries' MSVC check/clippy did pass; no Windows runtime claim is made. |
| Solid Edge build | Needs a Windows host with Solid Edge; no live proof exists. |
| Desktop bundle / installer | Blocked pending the packaging decision. |
| Manual adversarial walk | Not re-run; the boundary and worker-down paths are covered by the security suite and earlier `rust-e2e-qa` evidence. |
| CI artefact verification | No hosted run was inspected. |

---

## 3. Facts verified live, not taken from a report

| Claim now in the docs | How I checked it |
|---|---|
| The Rust front door builds FreeCAD through chat | a live `POST /api/studio/chat` with `cad_provider: "freecad"` returned the design above and `verified: true` |
| The FreeCAD chat values | the live response: `35571.416047 mm³`, `11889.101731 mm²`, 60 × 45 × 40, 5/5 checks, 568/586/1 |
| Solid Edge refuses on Linux with no fallback | a live chat turn returned the worker's sentence and `design: null` |
| No provider means no build | a live chat turn returned the provider-choice prompt and `design: null` |
| The cold-reload area fix works through the front door | stopped and restarted the front door on the same data dir; the design returned `cached: true` with `surface_area_mm2: 11889.101731` |
| The route split is 6 native / 10 proxied | `npm run check:native-readiness` output; also read `server.rs` and the crate README |
| The workspace is 418 tests | `cargo test --workspace --locked --release` and `-- --list` |
| `kala-studio` is 148 default / 164 all-features | workspace suite and the all-features suite |
| `kala-tools` is 229 | its release suite |
| The gateway is 55 accepted tests | 54 gateway test functions re-run green plus the accepted library doc-test |
| The live front-door differentials are 292/268/48/20 | all four harnesses re-run; all zero mismatches |
| The UI is 43 files / 619 tests | `npm test` |
| The front door health and provider probe | live `curl` against the release binary with its supervised worker |

Numbers that came from the cell reports rather than being re-derived:
the FreeCAD 28/28 corpus result and per-case assembly volumes
(`r2-chat-providers` / the FreeCAD provider cell), the gateway's 26-case
mock-provider differential and 3,324-entry corpus totals (`gateway-r1`),
the executor's 23-case corpus and registry/simulator 1,064 comparisons
(`tools-executor-r1`, `tools-registry-r1`), and the earlier
`rust-e2e-qa` screenshots/worker crash evidence.

---

## 4. Honest notes

1. **`kala-runtime` is only half-green at doc time.** The accepted gateway
   is green (54 test functions + its doc-test = 55). The crate is being
   extended by the in-progress `agent-r1` cell, and a crate-wide
   `cargo test --locked --release` currently **fails to compile**:
   `tests/agent_support/mod.rs` uses `Arc` without importing it
   (`E0425`/`E0433` in `agent_support`, then the `agent_differential`
   target). I did not edit it. This is recorded in `backend/README.md`,
   which states that the gateway is accepted and `agent-r1` is not.
2. **The corpus self-check is red.** `backend/compat/self_check.py` exits 1
   with differences in `cli_launch.json`, `manifest.json` and
   `studio_http.json`. The working tree carries uncommitted Studio edits
   and the corpus was not regenerated. I recorded the result rather than
   hiding it; no corpus file was modified.
3. **One flaky debug test was observed.** The first `cargo test --workspace
   --locked` debug run failed
   `the_chat_turn_is_journalled_through_the_worker` with 3 journal events
   instead of 1. The same test passed when run alone, and the full debug
   re-run passed. The release run never failed it. The tests use ephemeral
   ports, and this looks like a pre-existing parallel-test flake, not a
   docs change; it is noted here rather than silently ignored.
4. **The UI count moved from 614 to 619** because the working tree already
   contained concurrent Studio edits (the `docs-r1` report recorded 614).
   The docs now state the observed 619.
5. **`docs/studio-limitations.md` is shared with `docs-r1`.** Its git diff
   includes the earlier provider/geometry/journal work. My changes are the
   provider bullet, the cold-reload section, the test table and the
   not-run markings; I did not revert anything.
6. **No `docs/studio.md` edit.** It was already consistent with the new
   provider behaviour; its own claims were outside this cell's ownership.
7. **No browser or screenshot run.** The UI numbers come from `npm test`;
   the visible UI claims come from the existing Vellum/`rust-e2e-qa`
   evidence, and I did not drive the Electron shell.
8. **No runtime cutover.** The Python Studio remains the reference oracle.
   The docs continue to say that, and the front door is described as a
   migration artifact.
9. **No release packaging claim.** Readiness is READY for the current
   feature set (6/10, 0 missing), but packaging is still blocked by the
   decision on how the Python/OCP worker is shipped.

---

## 5. No-overclaims checklist

* [x] The false “Rust front door cannot build with FreeCAD or Solid Edge”
      limitation is **gone** from `docs/cad-providers.md` and
      `docs/studio-limitations.md`.
* [x] Chat and `POST /design` pass the chosen provider to the worker; the
      worker alone decides; there is **no fallback**.
* [x] FreeCAD through Rust chat is stated with the live differential
      evidence (292 cases, 0 mismatches) and the measured values.
* [x] Solid Edge is still described as **unproven**; Linux only refuses
      honestly. No Solid Edge build or Windows claim is made.
* [x] The cold-reload area fix is recorded as a **reader-only** Python
      worker fix, with the old-reader failure and the live cold restart.
* [x] The gateway is described as **accepted** at 55 tests; `agent-r1` is
      explicitly **in progress and not accepted**, with the compile caveat.
* [x] `kala-tools` is described as **229 tests including the executor**,
      with the 23-case executor and 1,064 comparison differentials.
* [x] The root workspace is **418**, not the stale 414.
* [x] `docs/rust-studio.md` no longer describes a nine-route service; it
      states 6 native / 10 proxied, 15+1 route-methods, 4 worker-only.
* [x] No Windows, COM, Solid Edge, installer, performance or runtime
      cutover claim was added.
* [x] No ADR, code, test, fixture or manifest was edited.
* [x] No command that was not run is presented as run; the not-run list in
      the rewritten front-door doc and this report matches.
* [x] No commit, push, deploy, delegation or credential read was performed.

---

## 6. Reproduction paths

The logs for every run are under `/tmp/opencode/docs-r2/`:

| File | Contents |
|---|---|
| `cargo-workspace.log`, `cargo-workspace-debug2.log` | release and final debug workspace runs |
| `cargo-studio-all.log` | 164-test all-features Studio run |
| `cargo-tools.log` | 229-test `kala-tools` run |
| `cargo-runtime-gateway-exact.log` | 54 gateway test functions; the accepted 55th is the doc-test |
| `cargo-kernel.log` | 152 `kala-kernel` tests |
| `cargo-fmt.log`, `cargo-clippy.log`, `cargo-build.log`, `cargo-build-studio.log` | format, clippy, workspace build and release front-door build |
| `windows-check.log`, `windows-clippy.log` | the five-crate MSVC compile checks |
| `pytest-unit.log`, `pytest-security.log`, `pytest-freecad.log` | 1555 / 189 / 8 Python results |
| `diff-worker.log`, `diff-python.log`, `diff-replay.log`, `diff-dto.log` | the four live differentials |
| `self-check.log` | the corpus self-check failure |
| `frontdoor-exact.log`, `chat-exact.json`, `chat-freecad.json`, `chat-solidedge.json`, `chat-noprovider.json`, `providers.json`, `design-after-restart.json` | live front-door evidence |
| `npm-typecheck.log`, `npm-test.log`, `npm-build.log` | UI gates |
