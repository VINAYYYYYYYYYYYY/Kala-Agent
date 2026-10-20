# Workspace consolidation cell — session log

- Date: 2026-10-19
- Task: `TASK.md` (T3 delegated task
  `rust-backend-20261019-workspace-r1-go-review48`)
- Model: OpenCode Go DeepSeek V4.1 Flash (variant max)
- No delegation, provider/model switch, commit, push or deploy.

## Starting state

- `backend/Cargo.toml` members: `kala-json`, `kala-persistence`; excludes:
  `kala-geometry`, `kala-budget`.
- `kala-geometry` and `kala-budget` had local locks; `kala-step` and
  `kala-artifacts` were standalone workspaces with local locks.
- `kala-studio` and `kala-contracts` were separate standalone workspaces with
  active writers and their own locks.
- Accepted baseline: 270 tests across the six crates.

## Timeline

1. Read `WAVE.md`, `SYNTHESIS.md`, `REVIEW.md`, and the accepted
   foundation/geometry/budget/persistence/deep-JSON/step/artifacts reports and
   reviews. Recorded pre-cell SHA-256 snapshots of all six crates' non-manifest
   files, the Python product/tests/fixtures, and the Studio/contracts trees in
   `/tmp/opencode/kala-workspace-r1/`.
2. Rewrote `backend/Cargo.toml`:
   - six members;
   - explicit `exclude` for `kala-studio`/`kala-contracts`;
   - shared dependency versions incl. `getrandom`, `libc`, `num-traits`,
     `rustix`, `windows-sys`;
   - single root release profile.
3. Converted geometry/budget/step/artifacts manifests to
   `[workspace.package]`/`[workspace.dependencies]` inheritance and removed
   their nested `[workspace]` roots and member profiles. Preserved every
   target-specific dependency and feature (`serde/derive`,
   `serde_json/arbitrary_precision`, `rustix` fs/std, windows-sys features).
4. Removed the four redundant generated crate locks after recording their
   hashes. Root `Cargo.lock` is now authoritative.
5. Ran `cargo metadata --offline` once to resolve the expanded graph (80 → 88
   packages). Cargo added `getrandom 0.3.4`, `r-efi 5.3.0`, `wasip2`,
   `wit-bindgen` and selected `syn 2.0.104` under the declared Rust 1.85
   compatibility filter. Verified `cargo metadata --locked --offline`
   afterward.
6. Ran the full verification matrix:
   - `cargo fmt --all --check` — pass;
   - debug and release `cargo test --workspace --locked -j2` — 270/270 pass,
     zero failures, zero ignored;
   - workspace all-target clippy with `-D warnings` — pass;
   - release workspace build — pass;
   - six per-crate metadata runs — pass, all report six workspace members;
   - `cargo tree` per target — Linux artifact build has `rustix`/`libc`,
     Windows has `windows-sys`, no cross-platform activation.
7. Windows checks:
   - whole workspace check fails in `libsqlite3-sys` bundled SQLite because
     `lib.exe` is unavailable (no MSVC toolchain);
   - the five pure-Rust members pass all-target `check` and `clippy
     -D warnings` for `x86_64-pc-windows-msvc`. No runtime claim.
8. Verified the active standalone workspaces without touching them:
   - `cargo metadata --locked --offline` passes in their real directories;
   - copied snapshot at 21:01:17 built with separate target dirs:
     Studio `cargo check` pass, contracts `cargo check` pass (writer warnings
     only). Their real targets and files were not touched.
9. Ran the Python corpus checks:
   - `self_check.py --skip-regenerate` passes all structural/live-reader
     counts;
   - full `self_check.py` fails only on pre-existing sidecar files
     (`sqlite3-shm/-wal`, created before this cell) and the live
     `source_state.git.tracked_changes` list in `manifest.json`; a field-by-field
     comparison found no other difference. No fixture was edited.
10. Updated `backend/README.md` and the six crate READMEs for the actual
    six-crate state, standalone Studio/contracts separation, toolchain
    honesty, commands, and platform/security boundaries.
11. Re-checked source integrity: 103 six-crate source/test files and 124
    Python files unchanged. Recorded Studio/contracts writer churn separately.
12. Wrote `REPORT.md` and this session log.

## Decisions

- **Remove redundant crate locks** rather than keep them as inactive
  references, because the brief allows either and a single lock is less
  error-prone. Hashes are in `REPORT.md`; the files are generated artifacts.
- **Keep `rust-version = "1.85"`** as the inherited declared floor, and state
  clearly that only 1.99.1 was tested. The alternative (bumping to 1.99.1)
  would have been a new MSRV claim not supported by this cell's evidence.
- **Exclude Studio/contracts explicitly** in the root manifest, in addition to
  their own `[workspace]` declarations, so root commands can never build them.
- **Build Studio/contracts from a copy with separate `CARGO_TARGET_DIR`s** so
  the verification does not race their writers' builds or mutate their trees.
- **Do not touch the pre-existing `-shm`/`-wal` fixture sidecars**; they are
  outside this cell's ownership and their presence is documented instead.

## Exact test evidence

| Run | Result |
|---|---|
| `cargo test --workspace --locked -j2` | 270 passed, 0 failed, 0 ignored |
| `cargo test --workspace --locked --release -j2` | 270 passed, 0 failed, 0 ignored |

Per crate: json 33, persistence 65, geometry 32, budget 65, step 34 + 1
doctest, artifacts 40. Live Python/OCP/FreeCAD oracles executed in both runs;
no skip environment variables were set.

## Handoff notes

- Root manifest/lock is stable and verified `--locked`.
- Studio/contracts cells should re-run their suites against the changed
  dependency manifests before their next acceptance; their locked metadata and
  a fresh-copy build passed here.
- Full corpus regeneration remains environment-dependent (dirty-path list and
  sidecar files); `--skip-regenerate` passes. This is a fixture-maintenance
  issue outside this cell, not a code change.
- No commit/push/deploy was performed.
