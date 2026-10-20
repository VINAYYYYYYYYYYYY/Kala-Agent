# Delivery repair session (R2)

## Starting state

Read `TASK.md`, `TASK-R2.md`, `REPORT.md`, `SESSION.md`, `REVIEW-1.md`,
`WAVE.md`, the design-turns wave/ADR-0009, the actual Electron resolver,
staging/packaging helpers, tests, workflow, Python Studio route/schema source
and native route table. Confirmed:

- REVIEW-1 P1: `stageNativeBackend` called `fs.rmSync(stageDir,{recursive:true})`
  before validation; arbitrary nonempty destinations were not ownership-bound.
- REVIEW-1 P2: `fs.cpSync(...,{dereference:true})` followed UI symlinks.
- Current Python router has 14 route-methods; native `allowed_methods` has 9.
- The concurrent `design-turns-20261019` wave owns Python/UI paths; treated
  read-only.

No live turns of the original delivery worker were used. No delegation,
model switch, commit, push, deploy or publish was performed.

## Implementation sequence

1. Added `studio/scripts/native-readiness.cjs`.
   - Parses Python `@router` decorators/schema classes and native
     `allowed_methods` arms.
   - Compares route-methods and DTO capability markers (`design_available`,
     chat design result, export `design_id`).
   - Produces a deterministic report with source digests and pending-feature
     labels; strict exit 1 while pending, `--report-only` exit 0.
   - Digests only route/DTO-defining Python files so unrelated design-module
     churn does not spuriously invalidate a stage, while native source is
     fully digested for rebuild staleness.
2. Replaced staging in `studio/scripts/native-backend.cjs`.
   - Destination policy: dedicated `studio/backend-native`, new/empty custom
     outside the repository, or previously helper-owned exact path; arbitrary
     nonempty/repo/control directories refused.
   - Ownership metadata is path-bound (`stageOwner`, `stageDir`, `managedBy`,
     `package`); legacy schema-1 default stage accepted only at the dedicated
     path.
   - Sibling candidate + validated swap + backup restore; only self-created
     candidate/backup ever removed.
   - `lstat` symlink guards for destination ancestors, binary and UI tree;
     explicit non-dereferencing copy; source-vs-copy hash comparison.
   - Metadata schema v2 records binary/static digests and source-parity
     snapshot. `verifyStageIntegrity` recomputes and compares them.
   - `assertStageUiMatches` compares the stage static digest with the current
     `studio/dist` build so a rebuilt UI cannot be packaged from an older
     stage.
   - Missing flag values error before mutation.
3. Updated `studio/scripts/desktop-pack.cjs`.
   - Uses `verifyStageIntegrity` + current readiness/source digest check.
   - Builds a packaging decision: `--dir` = labelled development artifact
     when pending; every release/NSIS invocation is refused while pending.
   - Writes `release/native-readiness.json`.
   - Removes all publish overrides and appends `--publish never`.
4. Changed Electron/launcher defaults.
   - Development default = repository `.venv` Python reference.
   - `KALA_STUDIO_DEV_BACKEND=native` = explicit native slice.
   - Invalid mode fails closed; packaged mode remains native-only.
   - Launcher builds native only in explicit native mode.
5. Updated tests.
   - New deletion sentinel/unmanaged directory test.
   - Ownership copy-to-different-path refusal.
   - Failed replacement preserves prior valid stage (bad binary, missing UI).
   - Symlinked UI root/component/asset and symlinked stage root rejection.
   - Corrupted binary/static digest refusal.
   - Missing argument values.
   - Current 14-vs-9 readiness, synthetic full-parity ready proof, release
     refusal vs labelled `--dir`, publish override stripping.
   - Default-reference/native-opt-in/invalid-mode resolver tests.
6. Updated `packaging/native_studio_smoke.py` to recompute static-tree
   SHA-256, reject symlinked stages and print the readiness snapshot.
7. Updated workflow, docs and READMEs; wrote this report set.

## Focused failures and fixes

- First staging probe failed with `ENOENT` because the candidate was created
  inside the not-yet-existing destination. Moved candidate/backup paths to
  siblings (`<stage>.staging-…`, `<stage>.backup-…`).
- First ownership test surfaced a misleading "metadata missing" error for an
  unmanaged directory. Wrapped metadata read so the refusal consistently says
  "unmanaged directory".
- `desktop-pack` test expected an explicit `validateNativeTree` reference;
  added the explicit call after integrity verification.
- TypeScript `noUncheckedIndexedAccess` on a test split; added explicit
  tuples.
- Full-suite run encountered unrelated concurrent UI work in
  `Inspector.tsx`/`VernierSlider` (typecheck error and two test failures).
  Those files are other-wave/user-owned and were not edited. After that wave
  settled, the same commands passed: typecheck clean and 435/435 tests.

## Actual verification

Run from `/home/v9i/Projects/Kala-Agent` and `studio/`:

```text
cargo fmt --all --check                                              exit 0
cargo clippy --offline --workspace --all-targets --locked -- -D warnings  exit 0
cargo test --offline --release --workspace --locked -j2              exit 0 (385 release tests + doc-tests)
cargo build --offline --workspace --release --locked                 exit 0
npm run typecheck                                                    exit 0
npm test                                                             435/435, 33 files
npx vitest run test/nativeBackendPackaging.test.ts test/electronMain.test.ts  57/57
npm run build                                                        exit 0
node scripts/native-readiness.cjs                                    exit 1 (expected pending)
node scripts/native-readiness.cjs --report-only                      exit 0
node scripts/native-backend.cjs --no-build                           exit 0 (stage v2)
npm run desktop:pack -- --dir                                        exit 0 (labelled dev artifact)
node scripts/desktop-pack.cjs --win nsis --x64                       exit 1 (release gate)
packaging/studio_smoke.py (from /tmp, child PATH="")                 PASS
packaging/native_studio_smoke.py (stage)                             PASS
packaging/native_studio_smoke.py (unpacked resources)                PASS
```

Runtime probes:

```text
# Default development resolver: .venv Python reference
health 200 {"status":"ok","design_available":true,"step_available":true}
GET /journal (bearer) 200
graceful shutdown: expected=true

# Python vs staged native drift
health: Python design_available=true / native field absent
GET  /journal:            Python 200 / native 404
GET  /journal/verify:     Python 200 / native 404
POST /design {}:          Python 422 / native 404
```

Staged artifact:
`studio/backend-native/kala-studio`, SHA-256
`5303450b23cbb6ad737d061a04d51e40b8facd0cc07a96c33427a93259aca7e9`,
6,941,744 bytes, static tree SHA-256 `5e7f4efb2e67e7c74e50af13cc97a03e1d51fbdf65a3eafcfcce6619ed75f4f8`; `native-build.json` schema v2, `nativeStep:true`, source
digests matching the final sources.

## Residual state

- Native ADR-0009 design/drawing/section/journal routes and design DTO fields
  remain unported. Release readiness is false by construction.
- REVIEW-1's 268-case/30-mismatch differential evidence remains pending; no
  mismatch was hidden, skipped or normalized.
- Windows/MSVC/NSIS execution and clean-host installer proof remain pending;
  the NSIS path is blocked until the expanded native port lands.
- No commit, push, deploy, publish, signing, credential read or live model
  call was made.
