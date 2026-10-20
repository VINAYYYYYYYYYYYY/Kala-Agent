# Delivery repair session (R3)

## Starting state

Read `TASK.md`, `TASK-R2.md`, `TASK-R3.md`, `REPORT.md`,
`SESSION-R2.md`, `REVIEW-1.md`, `REVIEW-2.md`, the actual staging/packaging
helpers, tests and installed electron-builder CLI/parser. Confirmed:

- REVIEW-2 P1: destination ownership/state was checked before several awaited
  hash operations, but the final swap moved and recursively deleted whatever
  was at the destination path. A probe that created
  `/tmp/kala-sol-stage-race-…/user-sentinel.txt` during the awaits lost it.
- REVIEW-2 P2: `decidePackaging` used `argv.includes('--dir')`; the installed
  parser yields `dir=false` for `--dir --dir=false` and `--no-dir`, so the
  same forwarded argv was an NSIS release invocation labelled development.
  `--dir --win nsis` is additionally a release invocation even with effective
  `dir=true`, because an explicit platform target replaces the unpacked
  default.
- Current Python router: 14 route-methods; native: 9. No ADR-0009 port or
  differential normalization is in scope.

The `design-turns-20261019` wave was read-only. No live child turns,
delegation, model/provider switch, commit, push, deploy, publish, signing,
credential read, live model call or Windows install was performed.

## Implementation sequence

1. `studio/scripts/native-backend.cjs`
   - Added `directoryStateDigest` (relative path/type/size/nanosecond mtime and
     small-file content, symlinks refused), `captureStageState`
     (device/inode identity + digest + nonempty), `sameStageState`
     and `assertStageStateUnchanged`.
   - Captured the destination before creating the candidate; nonempty
     destinations still require path-bound ownership metadata.
   - After candidate validation, re-captured the destination and refused any
     appeared/replaced/modified state. For existing managed stages, also
     re-ran `verifyStageIntegrity` and re-captured to narrow the async window.
   - After `rename(destination, backup)`, re-captured the backup and restored
     it if it was no longer the authorized snapshot.
   - Before `rmSync(backup)`, re-captured it again; on mismatch kept it and
     warned instead of deleting concurrent content.
   - Added optional deterministic `onBeforeInstall` and
     `onBeforeBackupRemoval` hooks for tests; the CLI passes none.
   - No `rmSync` of the destination remains.
2. `studio/scripts/desktop-pack.cjs`
   - Added `normalizeBuilderInvocation` using installed
     `electron-builder/out/builder.js`
     (`createYargs` + `configureBuildCommand` + `normalizeOptions`, strict,
     no process exit).
   - `unpackedOnly = dir === true && every normalized target type === 'dir'`.
   - `decidePackaging` now gates on `unpackedOnly`; invalid/ambiguous argv fails
     closed with the parser error. Effective targets are included in the
     refusal message.
   - `main` sanitizes `--publish` first and classifies the exact sanitized argv
     forwarded to electron-builder.
   - Existing `--publish never` forcing is unchanged; no upload API is called.
3. `studio/test/nativeBackendPackaging.test.ts`
   - New deterministic tests: destination created during awaits; authorized
     managed stage modified during awaits; managed stage replaced during
     awaits (both copies preserved); backup changed before cleanup is kept;
     pre-existing unknown top-level file in a managed stage refused and
     preserved.
   - Gate matrix for `--dir --dir=false`, `--dir --no-dir`,
     `--dir --win nsis`, `--dir=false`, `--no-dir`, conflicting repeats,
     platform/alias spellings and allowed effective dir-only forms.
   - Installed-parser parity test comparing `normalizeBuilderInvocation`
     against a direct `electron-builder/out/builder.js` parse for thirteen
     argv shapes; strict invalid-flag refusal.
4. Updated `docs/rust-studio.md` transaction/gate description.
5. Rebuilt UI, restaged native from the existing release binary, ran the
   packaged/native/Python smokes and focused/full suites.
6. Wrote this session log and the R3 `REPORT.md`.

## Focused failures and fixes

- The first version of the gate used `isDevelopmentDirectoryBuild` as the
  classification source; it still string-matched `--dir`. Replaced it with
  normalized effective targets after confirming with the installed parser
  that `--dir --win nsis` produces `windows:nsis` while `--dir` produces
  `<host>:dir`.
- The installed yargs parser needed `exitProcess(false)` so tests and the
  packaging helper do not terminate the process on invalid input; strict mode
  still throws and the gate converts it to a fail-closed error.
- TypeScript initially had no declarations for the new normalization/state
  APIs; added `desktop-pack.d.cts`/`native-backend.d.cts` entries.
- No test or accepted source needed weakening. No parity failure was rerun as
  a pass: readiness remains pending 14 vs 9.

## Actual verification

```text
npx vitest run test/nativeBackendPackaging.test.ts test/electronMain.test.ts
  -> 63/63 passed
npm test
  -> 441/441 passed, 33 files
npm run typecheck
  -> exit 0
npm run build
  -> exit 0
node scripts/native-backend.cjs --no-build
  -> exit 0 (managed stage replaced transactionally)
independent REVIEW-2 race probe
  -> refused; sentinelPreserved=true; no .staging-/.backup- leftovers
node scripts/desktop-pack.cjs --dir
  -> exit 0, development-unpacked-artifact, releaseEligible=false
node scripts/desktop-pack.cjs --dir --dir=false --win nsis
  -> exit 1, effective windows:nsis blocked
node scripts/desktop-pack.cjs --dir --no-dir --win nsis
  -> exit 1, effective windows:nsis blocked
node scripts/desktop-pack.cjs --dir --win nsis
  -> exit 1, effective windows:nsis blocked
node scripts/native-readiness.cjs --report-only
  -> exit 0, 14 vs 9, 5 routes + 3 capabilities pending
packaging/native_studio_smoke.py (stage, from /tmp)
  -> PASS
packaging/native_studio_smoke.py (linux-unpacked/resources/backend, from /tmp)
  -> PASS
packaging/studio_smoke.py (preserved Python backend, from /tmp)
  -> PASS, STEP 15518 bytes
cargo fmt --all --check --manifest-path backend/Cargo.toml
  -> exit 0
cargo test --offline --release --workspace --locked -j2
  -> exit 0, 389 test/doctest results
cargo clippy --offline --workspace --all-targets --locked -- -D warnings
  -> exit 0
```

Staged artifact: native binary SHA-256
`cb0b96d357c2715bc18033d24d96db9e6150fa7b7ae34980831adde90ed30662`
(6,963,584 bytes), static tree SHA-256
`32cd5a9856e0ed9e9227e1a47300c9e3ad1d3de46df30ba6d06660d217c9c695`,
`nativeStep:true`, source-parity digests matching the current sources.

## Residual state

- ADR-0009 native design/drawing/section/journal routes and three design DTO
  capabilities remain unported; release readiness is false by construction and
  NSIS remains blocked.
- REVIEW-1's 268-case/30-mismatch runtime differential remains pending for the
  expanded-port cell; it was not replayed, hidden or normalized here.
- Windows/MSVC/NSIS installer execution and clean-host proof remain pending;
  only workflow/parser/build definitions were checked on Linux.
- Portable directory compare-and-swap has a residual window inside the final
  rename syscall; the helper verifies immediately after the move and never
  recursively deletes an unauthorized directory.
- No commit, push, deploy, publish, signing, credential read or live model call
  was made.
