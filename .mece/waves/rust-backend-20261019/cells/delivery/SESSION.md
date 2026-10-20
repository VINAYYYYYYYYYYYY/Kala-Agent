# Delivery session log

## Start

Read `TASK.md`, `WAVE.md`, `SYNTHESIS.md`, workspace `REVIEW-1.md`, Studio
`TASK-R4/REVIEW-4`, the actual root manifests, `kala-studio` CLI/source
contracts, Electron main/scripts, electron-builder config, frozen smoke and
studio workflow. Confirmed the accepted Studio slice is a standalone crate
with its own workspace/lock/profile, and the Python reference/packaging paths
must stay untouched.

## Workspace integration

1. Added `crates/kala-studio` to `backend/Cargo.toml`; excluded only
   `crates/kala-contracts` and `crates/kala-process`.
2. Removed `[workspace]`, `[profile.release]` and the nested
   `crates/kala-studio/Cargo.lock`; kept the `native-step` feature seam and
   target-gated TLS dependencies.
3. Generated `backend/Cargo.lock` for seven members. Verified locked offline
   metadata resolves.
4. Ran `cargo test --offline --release --workspace --locked -j2`: 385 tests,
   exit 0. `serde_json` `preserve_order` feature unification was exercised;
   no source/test edits or skips were needed.
5. Ran `cargo test -p kala-studio --no-default-features`: 113 tests, exit 0.
6. Ran `cargo fmt --all --check`, all-target clippy with `-D warnings`, and
   the release workspace build: all exit 0.

## Native build/stage

Added `studio/scripts/native-backend.cjs`:

- `cargo build --manifest-path backend/Cargo.toml --package kala-studio
  --release --locked --target-dir backend/target`;
- argument arrays and `shell: false`; no npm `.cmd` shim;
- optional `--target`, `--no-build`, `--no-default-features`, `--stage-dir`,
  `--dist`;
- clean `studio/backend-native/` with binary + `studio_dist/` +
  `native-build.json`;
- protected-path guard for `studio/backend`, `studio/dist`, `backend/`,
  `packaging/`, `src/` and repo root;
- Python/OCP/vendor artifact rejection and symlink rejection;
- binary/tree SHA-256, toolchain, feature and commit metadata;
- a native-STEP marker guard so a no-default-features binary cannot be
  staged as a default package.

The first `--no-build` stage after a deliberate no-default-features test
picked up the wrong binary. The smoke immediately exposed
`step_available: false`; rebuilt with the default `npm run build:native`
(default features), re-staged and re-smoked. The guard above now fails that
case loudly instead of packaging it.

Updated Electron:

- packaged mode resolves only `resources/backend/kala-studio[.exe]`;
- development prefers `studio/backend-native/`;
- explicit `KALA_STUDIO_PYTHON` or `KALA_STUDIO_DEV_BACKEND=python` selects the
  Python reference; no arbitrary `python` fallback;
- token remains env-only; one backend per main process; window/IPC/security
  contract unchanged.

Updated `electron-builder.json` to copy only `backend-native/ ->
resources/backend/`. `desktop.cjs` may stage the native helper but never
spawns a backend. `desktop-pack.cjs` requires native metadata with
`nativeStep: true`.

## Packaging tests and checks

- `npm run typecheck`: final exit 0. (Intermediate runs were blocked by
  in-flight user edits to `SceneCanvas.tsx`/`DraftingSheet.tsx`, outside this
  cell's ownership; no frontend file was changed here.)
- `npm test` with the native stage: final 305 tests / 27 files passed. An
  intermediate run hit the user-owned drafting tool-palette assertion
  (4 vs 5) while that frontend work was still being edited; it passed after
  the user edit settled.
- New `test/nativeBackendPackaging.test.ts`: 12/12 including a real native
  child through `BackendProcess` with `PATH=""`, health and graceful shutdown.
- `npm run build`: Vite production build success.
- `npm run desktop:pack -- --dir`: success; unpacked
  `resources/backend` contained only `kala-studio`, `native-build.json` and
  `studio_dist/`.
- `packaging/studio_smoke.py` (unchanged) from `/tmp` with `PATH=""`: PASS
  health/UI/assets/preview/5,249-byte STEP.
- New `packaging/native_studio_smoke.py`: PASS on stage and unpacked layout
  (digest/layout, health, UI/assets, unauthenticated 401, preview, native
  STEP).
- Real Electron dev lifecycle: native resolved, health ok, cleanup
  `exited=true expected=true code=0`.
- Real Electron dev GUI launched under the host display, page target
  `Kala Studio` at the loopback URL, health ok, CDP close, Electron exit 0,
  no orphan backend.
- Real packaged Electron GUI from `release/linux-unpacked`: same result.

## Workflow and docs

Rewrote `.github/workflows/studio.yml` with native Windows and Linux dev jobs,
absolute `KALA_*_PYTHON` oracle paths, MSVC/ring/NASM expectations, native
staging/smokes with Python-free child PATH, unsigned NSIS upload, and the
retained Python reference/FreeCAD adapter jobs. No job signs, publishes,
deploys, installs, or calls a provider.

Wrote `docs/rust-studio.md` and updated `backend/README.md`,
`kala-studio/README.md` and the delivery report. No Windows/hosted run,
installer execution, COM/Solid Edge proof, benchmark or full-migration claim
is made. Python reference/artifacts remain intact.

Concurrent user/other-cell changes were present in `studio/package-lock.json`,
the React frontend and Python `src/kala_agent/studio/**`; this cell did not
edit them. The only `package.json` change is the `build:native` script, so no
lock update was required. The shared-worktree status was left untouched.
