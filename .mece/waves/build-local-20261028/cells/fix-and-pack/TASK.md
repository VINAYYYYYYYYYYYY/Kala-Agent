# Cell fix-and-pack: fix the Linux type/test breakage from 66465c0, then build and smoke the AppImage

Read `../../WAVE.md` first (the blockers and the rules).

## 1. Fix (smallest correct change; no behaviour change to product code)
- **`studio/electron/main.d.cts`.** Declare what `main.cjs` actually implements now: `nonce` (and `requireLaunchNonce`, `logger` if missing) on `BackendProcessOptions`, `nonce` on `BackendInvocation`, plus any other export the new tests use. Read `main.cjs` to match the real shapes, and do not change `main.cjs`.
- **`studio/test/electronMain.test.ts` and `studio/test/freecadVendorHashes.test.ts`.** Fix the strict-typing errors (narrow the possibly-undefined values with an explicit assertion; omit `hashes` instead of passing `undefined`). Keep every assertion's meaning.
- **`tests/unit/test_private_files.py`.** Make the stub accept any arguments (`def refuse(*_args, **_kwargs)`), so it raises OSError on both POSIX (`os.chmod(path, mode)`) and Windows (`restrict_windows_acl(path)`). Do not change `src/kala_agent/private_files.py`.

## 2. Gates
- In `studio/`: `npm run typecheck` (0 errors), `npm test` (all pass; baseline 794), `npm run build`.
- `.venv/bin/python -m pytest -q -p no:cacheprovider tests/unit tests/security`: all pass. Baseline: 1866 passed + this one fixed, 2 skipped.

## 3. Build and smoke the AppImage
- Run `packaging/linux/build-appimage.sh` (read `packaging/linux/README.md` first). It builds the native backend and the UI, runs electron-builder, and runs the headless smoke `packaging/linux/smoke-appimage.mjs`.
- The release backend must be built from the current working tree (HEAD `a31229a` plus your fixes). Make sure the native stage is rebuilt from source, not reused from a stale stage.
- The smoke must PASS: the OCCT + FreeCAD L-bracket, exact; the section; and the STEP round trip. Save the full log to `cells/fix-and-pack/package.log`.
- Report the AppImage path, size in bytes and sha256.

## REPORT.md
Write it here, with:
- the exact diffs (or a summary per file);
- the gate counts;
- the smoke verdict, with the key log lines;
- AppImage path, size and sha256;
- anything you could not fix.

No commits. Clean up your temp dirs.
