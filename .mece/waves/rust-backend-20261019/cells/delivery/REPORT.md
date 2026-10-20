# Native Studio delivery repair — destination-state binding and effective builder gate (R3)

## Outcome

This round repairs the two independent REVIEW-2 reproductions without changing
accepted Rust/Python/UI source, the seven-crate workspace integration, the
Python-reference development default, or the pending current-feature release
gate:

- **P1 (staging): destination identity/state is bound across the whole async
  stage transaction.** The destination is snapshotted before any temporary
  work (device/inode plus a recursive entry digest). Immediately
  before the swap the same snapshot must still be present. A directory created,
  replaced or modified while the candidate was being hashed is refused and
  left untouched. The directory moved to the backup path is re-verified after
  the rename; destructive backup cleanup only happens when it still matches
  the authorized snapshot.
- **P1 (release gate): classification uses the effective installed
  electron-builder arguments.** The gate invokes electron-builder's own
  `createYargs`/`configureBuildCommand`/`normalizeOptions`, so
  `--dir --dir=false --win nsis`, `--dir --no-dir --win nsis` and
  `--dir --win nsis` are all treated as NSIS/release invocations and blocked
  while current-feature parity is pending. Only effective `dir`-only targets
  produce the labelled unpacked development artifact.

The accepted native slice remains the original nine `/api/studio` route-methods.
The working Python reference (current 14 route-methods, ADR-0009
design/drawing/section/journal) remains the development default and is intact.
No stubs, fake design availability, hidden Python product fallback, source
deletion, accepted-test weakening or parity normalization were added.

No commit, push, deploy, publish, signing, credential use, live provider call,
Windows install or hosted workflow run was performed. The concurrent
`design-turns-20261019` wave was read-only and untouched.

## P1 repair — staging transaction binds destination state

`studio/scripts/native-backend.cjs` now:

1. Captures `{ exists, device/inode identity, recursive state
   digest, nonempty }` for the destination **before** creating the candidate.
   A nonempty destination must still carry validated path-bound ownership
   metadata (`stageOwner`, `managedBy`, `package`, `stageDir`); the legacy
   schema-1 stage is accepted only at the dedicated default path.
2. Builds and validates the candidate in a sibling
   `${stageDir}.staging-<pid>-<time>-<random>` directory. The state digest
   covers every entry (relative name, type, size, nanosecond mtime, and content
   of small files such as `native-build.json`, `index.html`); large binary/UI
   content is bound by `verifyStageIntegrity`. A managed stage must also have
   exactly the helper-written top-level layout (`kala-studio[.exe]`,
   `studio_dist/`, `native-build.json`); an unknown pre-existing top-level file
   is refused and preserved rather than swept away with the stage.
3. Immediately before installation, re-captures the destination and refuses if
   it is not byte-for-byte the authorized state. A destination that appeared
   (the REVIEW-2 probe), disappeared, was replaced, or gained a file is left
   untouched; the candidate is discarded.
4. For an existing managed stage, also re-runs `verifyStageIntegrity` on the
   destination before moving it, then re-captures the state to narrow the
   async verification window. This refuses an in-place rewrite that preserved
   size/mtime.
5. Renames the destination to a unique sibling backup and re-captures the
   backup. If it is not the authorized snapshot, the backup is renamed back and
   the stage is refused; nothing is deleted. Only then is the candidate
   renamed into place.
6. Before deleting the backup, re-captures it again. Only a matching
   authorized snapshot is recursively removed. If it changed after the move,
   the new stage stays installed, the backup (including any concurrent file)
   is preserved, and the path is reported for manual inspection.
7. `discardCandidate` only removes the helper-created candidate path. There is
   no `rmSync(destination)`, no deletion of an unmanaged/custom/project path,
   and no deletion of a pre-existing candidate/backup path.

Optional `onBeforeInstall` / `onBeforeBackupRemoval` hooks are used only to
make the two races deterministic in tests; the CLI does not set them.

### Transaction guarantees and limits

- **Crash/failure before the swap:** previous stage untouched; only the
  helper's candidate is removed.
- **Destination changed before the swap:** previous/concurrent directory
  untouched; candidate removed; staging fails.
- **Destination changed between the pre-swap check and the rename syscall:**
  the renamed directory is verified against the snapshot immediately after the
  move and restored if it differs. The remaining window is inside the
  platform rename syscall itself; Node exposes no `renameat2(RENAME_NOREPLACE)`
  binding. No recursive deletion occurs in that window.
- **Destination absent and a nonempty directory appears in the final syscall
  window:** POSIX/Windows directory rename onto a nonempty directory fails; the
  candidate is discarded and concurrent files remain. An empty directory may
  be replaced by the rename (no files are lost). This residual is documented,
  not hidden.
- **Backup changed after the move:** preserved, never deleted.
- Windows directory rename semantics are covered by code paths but remain
  unexecuted on a Windows host (see residuals).

## P1 repair — release gate follows effective builder mode

`studio/scripts/desktop-pack.cjs`:

- `normalizeBuilderInvocation(argv)` loads the installed
  `electron-builder/out/builder.js` and parses the **forwarded** argv with
  `configureBuildCommand(createYargs())`, then calls electron-builder's
  `normalizeOptions`. It reports effective `dir`, platform target lists and
  normalized target types.
- `unpackedOnly` is true only when effective `dir === true` and every
  normalized target is electron-builder's `dir` target.
- `decidePackaging` uses `unpackedOnly`, not `argv.includes('--dir')`.
  Pending parity therefore blocks any installer target, including:
  - `--dir --dir=false --win nsis` (effective `dir=false`, Windows NSIS)
  - `--dir --no-dir --win nsis` (effective `dir=false`, Windows NSIS)
  - `--dir --win nsis` (effective `dir=true`, but target is NSIS; `--dir` only
    supplies the default when no explicit platform target is given)
  - `--dir=false`, `--no-dir`, conflicting/repeated negations
  - invalid/unknown flags (strict parse fails closed)
- Allowed development invocations are the effective unpacked target only:
  `--dir`, `--dir=true`, repeated `--dir`, `--dir --win` and
  `--dir --win dir`. They write `release/native-readiness.json` with
  `classification: development-unpacked-artifact`,
  `releaseEligible: false` and a non-distribution warning.
- `main` sanitizes publication first and passes the exact sanitized argv that
  is forwarded to electron-builder's gate, then appends one
  `--publish never`. No upload API is invoked by the helper or tests.

If a future environment has readiness `true`, effective installer targets
classify as `release`; effective dir-only targets still classify as a
development artifact.

## Current-feature drift remains the release blocker

Actual source-derived evidence (unchanged this round):

- Python router: **14** route-methods.
- Native `allowed_methods`: **9** route-methods.
- Missing native routes:
  - `POST /api/studio/design`
  - `GET /api/studio/design/{id}/drawing`
  - `GET /api/studio/design/{id}/section`
  - `GET /api/studio/journal`
  - `GET /api/studio/journal/verify`
- Missing native DTO capabilities:
  - `health.design_available`
  - `chat.design_result`
  - `export.design_id`

REVIEW-1's accepted **268 live differential cases / 30 mismatches** remain the
pending expanded-port evidence. This round neither replays nor normalizes them;
the source-derived readiness check is a missing-feature guard, not behavioral
parity proof.

## Development and packaged resolution (unchanged this round)

| Mode | Backend |
|---|---|
| Packaged | `resources/backend/kala-studio[.exe]` only; Python overrides ignored. Release generation blocked by the readiness gate. |
| Development default | Repository `.venv` Python reference (`-m kala_agent.studio`, `PYTHONPATH=<repo>/src`); no arbitrary PATH fallback. |
| Development native (explicit) | `KALA_STUDIO_DEV_BACKEND=native`; missing stage gives `npm run build:native` instructions. |
| Development reference (explicit) | `KALA_STUDIO_DEV_BACKEND=python` or a specific `KALA_STUDIO_PYTHON` interpreter. |
| Invalid mode | Fails closed. |

The token remains env-only; the one-backend lifecycle, argv contract,
readiness probe, IPC sender validation, CSP, navigation denial and sandbox
settings are unchanged.

## Changed paths (R3)

- `studio/scripts/native-backend.cjs` — destination state snapshot/binding,
  exact helper-owned top-level layout check, post-rename backup verification,
  cleanup re-verification, test hooks.
- `studio/scripts/native-backend.d.cts` — hook/state declarations.
- `studio/scripts/desktop-pack.cjs` — installed-parser/normalized-target gate,
  fail-closed ambiguous arguments, sanitized-argv classification.
- `studio/scripts/desktop-pack.d.cts` — normalization declarations.
- `studio/test/nativeBackendPackaging.test.ts` — deterministic concurrent
  destination-created/replaced/modified refusals, pre-existing unknown
  top-level file refusal, backup-preservation test, installed-parser target
  matrix, negated/conflicting `--dir` gate matrix.
- `docs/rust-studio.md` — transaction and effective-target gate documentation.
- `.mece/waves/rust-backend-20261019/cells/delivery/REPORT.md`,
  `SESSION-R3.md` — this report and session log.

No root manifest/lock, crate manifest/lock, Electron main, launcher, workflow,
Python source/test, frontend source, adapter, fixture, contracts/process crate
or other-wave file was changed this round.

Generated/ignored from verification: `studio/backend-native/`,
`studio/release/`, `studio/dist/`, root Cargo `target/`.

## Checks actually run

Linux development host:

| Check | Result |
|---|---|
| `npx vitest run test/nativeBackendPackaging.test.ts test/electronMain.test.ts` | **63/63 passed** |
| `npm test` | **441/441 passed, 33 files** |
| `npm run typecheck` | exit 0 |
| `npm run build` | exit 0; Vite production bundle |
| `node scripts/native-backend.cjs --no-build` | exit 0; managed stage replaced transactionally |
| Independent REVIEW-2 race probe (destination created during hash awaits) | refused; `sentinelPreserved:true`; no staging/backup leftovers |
| `node scripts/desktop-pack.cjs --dir` | exit 0; labelled `development-unpacked-artifact`, `releaseEligible:false` |
| `node scripts/desktop-pack.cjs --dir --dir=false --win nsis` | exit 1; effective `windows:nsis` blocked |
| `node scripts/desktop-pack.cjs --dir --no-dir --win nsis` | exit 1; effective `windows:nsis` blocked |
| `node scripts/desktop-pack.cjs --dir --win nsis` | exit 1; effective `windows:nsis` blocked |
| `node scripts/native-readiness.cjs --report-only` | exit 0; 14 vs 9, 5 routes + 3 capabilities pending |
| `packaging/native_studio_smoke.py --stage-dir studio/backend-native` from `/tmp` | PASS; health/UI/auth/preview/native STEP, child PATH free of Python |
| `packaging/native_studio_smoke.py` on `release/linux-unpacked/resources/backend` | PASS; packaged layout |
| `packaging/studio_smoke.py --backend studio/backend/kala-studio` from `/tmp` | PASS; preserved Python reference binary (STEP 15,518 bytes) |
| `cargo fmt --all --check --manifest-path backend/Cargo.toml` | exit 0 |
| `cargo test --offline --release --workspace --locked -j2` | exit 0; 389 reported test/doctest results (accepted suite unchanged) |
| `cargo clippy --offline --workspace --all-targets --locked -- -D warnings` | exit 0 |

Staged artifact:

- `studio/backend-native/kala-studio`, SHA-256
  `cb0b96d357c2715bc18033d24d96db9e6150fa7b7ae34980831adde90ed30662`,
  6,963,584 bytes.
- `studio_dist/` tree SHA-256
  `32cd5a9856e0ed9e9227e1a47300c9e3ad1d3de46df30ba6d06660d217c9c695`
  (rebuilt after the current Vite bundle).
- `nativeStep:true`; `pythonSourceSha256`
  `b8c31fb074a59563e9c78e86c46039a575255988539b5fb006d1e9c463ea73a1`;
  `nativeSourceSha256`
  `37add0e8c7adc7c215795d676ee236c58112ce632180d610d76e9a2e7009bd7a`.

## Preserved

- Accepted Rust crate source/tests, root manifests/lock and workspace
  membership unchanged.
- Python Studio source/tests, frozen Python binary (`studio/backend/`) and
  packaging sources unchanged; the real Python smoke still passes.
- Design-turn wave untouched and still the development default.
- No accepted test/fixture weakened; no current-Python differential mismatch
  removed or normalized.
- Existing desktop IPC/security tests retained and passing.

## Residuals / unresolved expanded-port boundary

1. **Native ADR-0009 design/journal port remains pending.** Five route-methods
   and three DTO capabilities above need real compiled/measured/journal
   semantics and compatibility gates before release readiness can become true.
   No stubs or fake capabilities were added.
2. **REVIEW-1's 30 differential mismatches remain unresolved by design.** The
   expanded-port cell must replay and reduce them; the readiness gate is not
   behavioral parity proof.
3. **Windows/MSVC/NSIS and clean-host execution are unproven.** The workflow is
   a definition, not a hosted run. This round's Windows-relevant checks are
   parser/build/staging definitions only; no Windows host or installer was
   executed. NSIS remains blocked by readiness until item 1 lands.
4. **Rename syscall residual window.** The helper binds state before and
   verifies immediately after the rename, but cannot atomically compare-and-
   swap a directory portably. A nonempty concurrent directory in the final
   syscall window makes the installation rename fail rather than being
   deleted; an empty concurrent directory may be replaced.
5. **GUI execution was not repeated.** Previous worker GUI evidence remains
   worker evidence; this round used real native process/smoke/lifecycle probes.
6. Linux remains development/testing only; no Linux Solid Edge or product
   release claim is made.
