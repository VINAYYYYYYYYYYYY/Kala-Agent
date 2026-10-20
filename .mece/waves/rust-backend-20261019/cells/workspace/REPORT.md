# Workspace consolidation cell — report

- Cell: `.mece/waves/rust-backend-20261019/cells/workspace`
- Brief: `TASK.md` in this directory
- Implementer: OpenCode Go **DeepSeek V4.1 Flash (variant max)**
- Date: 2026-10-19
- No delegation, model/provider switch, commit, push, deploy, cutover or
  benchmark. Sole root manifest/lock owner this round.

## 1. Outcome

The six accepted crates are now one root `backend/` workspace with one
authoritative `backend/Cargo.lock`:

```
kala-json  kala-persistence  kala-geometry  kala-budget  kala-step  kala-artifacts
```

`kala-studio` and `kala-contracts` remain separate standalone workspaces and
are now **explicitly excluded** in the root manifest. No file under either
directory was edited by this cell.

Accepted source, tests, fixtures and public APIs were not changed. All **270**
accepted tests (269 test functions + 1 doctest) pass in both debug and release
with zero failures and zero ignored tests.

## 2. Ownership and changed paths

Owned and changed:

| Path | Change |
|---|---|
| `backend/Cargo.toml` | six members; `exclude = ["crates/kala-studio", "crates/kala-contracts"]`; shared dependency versions; one root release profile |
| `backend/Cargo.lock` | regenerated as the single authoritative lock (80 → 88 packages) |
| `backend/README.md` | rewritten for the actual six-crate capability set, standalone Studio/contracts split, toolchain limits, commands and proof boundaries |
| `backend/crates/kala-json/README.md` | new crate README (manifest/lock/source unchanged) |
| `backend/crates/kala-persistence/README.md` | new crate README (manifest/lock/source unchanged) |
| `backend/crates/kala-geometry/Cargo.toml` | workspace inheritance and centralized versions; member `[profile.release]` removed |
| `backend/crates/kala-geometry/README.md` | membership/commands updated |
| `backend/crates/kala-geometry/Cargo.lock` | removed as redundant generated lock (sha256 `87b9e821…cdd619`) |
| `backend/crates/kala-budget/Cargo.toml` | workspace inheritance and centralized versions; profile removed |
| `backend/crates/kala-budget/README.md` | new crate README |
| `backend/crates/kala-budget/Cargo.lock` | removed as redundant generated lock (sha256 `d9936cd9…ffaf8e`) |
| `backend/crates/kala-step/Cargo.toml` | nested `[workspace]` removed; workspace inheritance/centralized versions; profile removed |
| `backend/crates/kala-step/README.md` | membership/commands updated |
| `backend/crates/kala-step/Cargo.lock` | removed as redundant generated lock (sha256 `60f8c8f2…d2b50cd`) |
| `backend/crates/kala-artifacts/Cargo.toml` | nested `[workspace]` removed; workspace inheritance/centralized versions; target-specific deps preserved; profile removed |
| `backend/crates/kala-artifacts/README.md` | membership and test commands updated |
| `backend/crates/kala-artifacts/Cargo.lock` | removed as redundant generated lock (sha256 `631cb184…971350`) |
| `.mece/waves/rust-backend-20261019/cells/workspace/REPORT.md` | this report |
| `.mece/waves/rust-backend-20261019/cells/workspace/SESSION.md` | session log |

Not changed: every `src/`, `tests/`, `examples/`, `tools/` file of the six
crates; all fixtures and `compat/`; all Python product/tests; `kala-studio`;
`kala-contracts`; `.gitignore` (existing `**/target` is sufficient).

`kala-json` and `kala-persistence` manifests were already workspace-inheriting
and needed no change; their READMEs were added.

## 3. Manifest and lock consolidation

### Root manifest

- Members: the six accepted crates only.
- Excludes: `crates/kala-studio`, `crates/kala-contracts`, which each declare
  their own `[workspace]` and lock.
- `resolver = "3"`, `edition = "2024"`.
- Shared versions: existing `chrono`, `num-bigint`, `parking_lot`, `rusqlite`,
  `ryu`, `serde`, `serde_json`, `sha2`, `tempfile`, `thiserror`; added
  `getrandom = "0.3.4"`, `libc = "0.2"`, `num-traits = "0.2"`,
  `rustix = "1.1.5"`, `windows-sys = "0.61.2"`.
- Member feature sets are unchanged: `serde/derive` only where previously
  used; `serde_json/arbitrary_precision` on geometry, budget and step (and
  kala-json's dev dependency); persistence keeps plain `serde_json`.
- Target-scoped dependencies stay target-scoped: `libc`/`rustix` only on
  `cfg(unix)`, `windows-sys` only on `cfg(windows)`.
- One root `[profile.release] strip = "symbols"`; member copies removed.
- Package metadata for geometry/budget/step/artifacts now inherits
  `version/edition/rust-version/license/publish` from `[workspace.package]`.

### Unified lock

`cargo metadata --offline` (one-time, then `--locked`) produced an 88-package
lock. Changes relative to the old 80-package root lock:

- Added workspace packages: `kala-geometry`, `kala-budget`, `kala-step`,
  `kala-artifacts`.
- Added `getrandom 0.3.4` (artifact store) alongside the pre-existing
  `getrandom 0.4.3` (tempfile); added `r-efi 5.3.0`, `wasip2 1.0.1+wasi-0.2.4`
  and `wit-bindgen 0.46.0` required by that getrandom version. These are
  target-specific resolution entries; `cargo tree` for Linux and Windows shows
  they are not built on either.
- `syn 2.0.119 → 2.0.104` for proc-macro dependencies. Cargo reported
  "Locking 5 packages to highest Rust 1.85 compatible versions"; this is the
  resolver honoring the inherited `rust-version = 1.85`, not a source change.
- No other locked version changed. All tests were re-run after this resolution.

`cargo metadata --locked --offline` passes, and four now-redundant
standalone locks were removed. Final lock SHA-256:
`58a5df05940418eaff640f6dddf230abcadfe1dd36c9cb3d1ce31366ddda5d69`.
`kala-studio` and `kala-contracts` keep their
own locks untouched (see §7; the contracts writer updated its own lock during
this round, not this cell).

## 4. Verification actually executed

All Rust commands from `backend/`, Rust 1.99.1, Linux x86-64. `-j2` was used
to limit contention with other active workers.

| Command | Result |
|---|---|
| `cargo fmt --all --check` | pass (run before and after documentation edits) |
| `cargo test --workspace --locked -j2` (debug) | **270 passed, 0 failed, 0 ignored** |
| `cargo test --workspace --locked --release -j2` | **270 passed, 0 failed, 0 ignored** |
| `cargo clippy --workspace --all-targets --locked -j2 -- -D warnings` | pass |
| `cargo build --workspace --release --locked -j2` | pass |
| `cargo metadata --locked --offline --format-version 1` | pass; exactly six workspace members; Studio/contracts absent from packages |
| Per-crate `cargo metadata --manifest-path crates/<crate>/Cargo.toml --locked --offline --no-deps` (six runs) | all exit 0; `version=0.1.0`, `edition=2024`, `rust_version=1.85`, six workspace members; targets resolved |
| `cargo tree -p kala-artifacts --target x86_64-unknown-linux-gnu` | `libc`/`rustix` present, no `windows-sys` |
| `cargo tree -p kala-artifacts --target x86_64-pc-windows-msvc` | `windows-sys` present, no `libc`/`rustix` |
| `.venv/bin/python backend/compat/self_check.py --skip-regenerate` | pass; 66 JSON, 5 chain events, 85 Decimal, 48 Studio HTTP, 49 core HTTP, 6 geometry meshes |
| `.venv/bin/python backend/compat/self_check.py` | **fails only on environmental comparisons**, see §8 |

### Test inventory (debug and release identical)

| Crate | Tests |
|---|---:|
| `kala-json` | 33 |
| `kala-persistence` | 65 (`kala-json` + `kala-persistence` = 98, matching deep-JSON review) |
| `kala-geometry` | 32 |
| `kala-budget` | 65 |
| `kala-step` | 34 + 1 doctest = 35 |
| `kala-artifacts` | 40 |
| **Total** | **270** (269 functions + 1 doctest) |

This is exactly the accepted baseline; no test was added, removed, renamed or
filtered. `kala-artifacts` re-executes its own test binary for 4 process-worker
runs; those child harness lines are not additional tests. Live Python/OCP/
FreeCAD differentials ran as part of both workspace runs (no skip variables
were set), including the 28-case OCP and 6-file FreeCAD checks.

## 5. Windows checks (compile only — no runtime claim)

- **Whole workspace**:
  `cargo check --workspace --all-targets --locked --offline --target x86_64-pc-windows-msvc -j2`
  fails while building `libsqlite3-sys 0.38.2` (`bundled` SQLite):
  `error occurred in cc-rs: failed to find tool "lib.exe": No such file or directory`.
  No MSVC `cl.exe`/`lib.exe` toolchain is installed here. `kala-persistence`
  therefore remains unverified for Windows compilation; no runtime was altered.
- **Pure-Rust crates**:
  `cargo check` and `cargo clippy -- -D warnings`, both with
  `--all-targets --target x86_64-pc-windows-msvc`, pass for
  `kala-json`, `kala-geometry`, `kala-budget`, `kala-step` and
  `kala-artifacts`. This includes the Windows-only artifact tests and
  `sys/windows.rs`; it does **not** execute them.
- No Windows host, Solid Edge COM runtime, MSVC link or Windows execution
  evidence exists in this cell.

## 6. Toolchain / MSRV honesty

Every check used `cargo 1.99.1` / `rustc 1.99.1`. The workspace still declares
`edition = "2024"` and inherited `rust-version = "1.85"`; that is a declared
floor, not a tested claim. No Rust 1.85 toolchain was installed, so no 1.85
build or test is claimed. The README states this explicitly.

## 7. Active standalone Studio and contracts

No file under either crate was edited. Their locked resolution still works:
`cargo metadata --locked --offline --no-deps` succeeded in both actual crate
directories immediately after the manifest changes.

Because those cells are actively writing, a full build was done from a
timestamped copy (21:01:17) using separate `CARGO_TARGET_DIR`s, so their build
locks/targets were not raced and their working trees were untouched:

| Snapshot command | Result |
|---|---|
| `cargo check --locked --offline` in copied `kala-studio` | pass (14.85 s) |
| `cargo check --locked --offline` in copied `kala-contracts` | pass (5.64 s); 13 warnings from the writer's in-progress source, none from the shared dependency manifests |

Both snapshots consumed the modified `kala-json` / `kala-geometry` /
`kala-step` / `kala-budget` manifests by path. Their own locks did not need
re-resolution.

**Dependency changes to report to the Studio/contracts cells:**

1. `kala-json`, `kala-geometry`, `kala-step`, `kala-budget` and
   `kala-artifacts` now inherit package fields and dependency versions from
   `backend/Cargo.toml` and are members of the root workspace. They are still
   plain path dependencies for the standalone workspaces.
2. Their dependency versions/features are unchanged in resolution except the
   root lock's `syn 2.0.104` selection; the standalone workspaces keep their
   own locks and were not modified.
3. `kala-step` and `kala-artifacts` no longer declare nested `[workspace]`
   roots or local locks. Path-dependency use from Studio/contracts is
   unaffected and was build-checked above.

During this cell the active writers also changed files under `kala-contracts`
(`Cargo.toml`, `Cargo.lock`, README, `src/{construction,lib,scalar,wire}.rs`)
and `kala-studio/src/settings.rs`. Those are their changes, observed after this
cell's baseline hash snapshot; this cell made no writes there.

## 8. Failures and environmental conditions

1. **`compat/self_check.py` full regeneration exits 1** for two pre-existing
   environmental reasons, not corpus/product drift:
   - The checked-in fixture directory contains `schema_v2.sqlite3-shm` and
     `schema_v2.sqlite3-wal` created at 16:21 (before this cell, by earlier
     tests); the clean regeneration does not create them, so the file-set
     comparison lists exactly those two extras.
   - `manifest.json` differs **only** in
     `source_state.git.tracked_changes`: the live repository now has additional
     unrelated dirty paths (60 vs the frozen 19), while the commit, Python
     source hashes and all other manifest content match. A field-by-field
     comparison found no other difference.
   Structural/live-reader self-check with `--skip-regenerate` passes all
   counts listed in §4. No fixture or manifest file was edited.
2. **Whole-workspace Windows check** fails at bundled SQLite as described in
   §5; the five pure-Rust members pass.
3. No required Linux check failed. No test was skipped or ignored.

## 9. Integrity evidence

- 103 files under the six crates' `src/`, `tests/`, `examples/`, `tools/`
  matched their pre-cell SHA-256 values exactly.
- 124 Python source/test/fixture files matched their pre-cell SHA-256 values
  exactly.
- The only differences under `kala-studio`/`kala-contracts` versus the
  pre-cell snapshot are the files listed in §7, all written by those cells'
  active workers after the snapshot.
- No commit/push/deploy/cutover; no benchmark or performance claim.

## 10. Remaining work (not this cell)

Typed domain repositories and provenance migration; secure Windows artifact
runtime proof; contracts acceptance; Studio HTTP acceptance/integration;
core HTTP/CLI; kernel/policy/workflow/memory/tools/runtime; native Solid Edge
COM execution; packaging cutover; controlled release benchmarks on a Windows
host. The six-crate workspace is a consolidated library foundation, not a
complete backend.
