# fix-and-pack — REPORT

Tree: `main` HEAD `a31229a`, working tree plus the four tracked edits below (no commits).
Date: 2026-10-30. Host: Linux x86-64, Node v26.8.2, npm 11.19.1, cargo/rustc 1.99.1, FreeCADCmd 1.1.4 (Python 3.14.8 / cp314).

## Verdict

All gates pass and the packaged AppImage passes the strict smoke
(`--require-geometry --require-freecad`): OCCT and FreeCAD L-brackets exact,
section exact, both STEP round trips equal.

| | |
|---|---|
| AppImage | `/home/v9i/Projects/Kala-Agent/studio/release/kala-studio-0.1.0-x86_64.AppImage` |
| Size | **163 682 196 bytes** (156.1 MiB) |
| SHA-256 | **`a35d377a3e9d8a997952ac690ecc0f357779135351c0bd0c398b37b392b2e7f4`** |
| Built | 2026-10-30 05:43 local, from HEAD `a31229a` + these fixes (native stage rebuilt from source) |
| Stage binary SHA-256 | `019ff86aa5091e596499cdf70c5b37630ba1b969129e3964e1d9cd31bf836c84` |

Full build + smoke log: `package.log` (this directory).

## 1. Fixes (exact diffs)

### `studio/electron/main.d.cts` — declare the launch nonce that `main.cjs` already implements

`main.cjs` was not touched.

```diff
@@ export interface BackendInvocation
   env: NodeJS.ProcessEnv;
   port: number;
   token: string;
+  nonce: string | null;
 }

@@ export interface BackendProcessOptions
   env: NodeJS.ProcessEnv;
   port: number;
   token: string;
+  nonce?: string | null;
+  requireLaunchNonce?: boolean;
   logger?: RotatingLogInstance | null;
 }
```

`nonce` is always returned by `buildBackendInvocation` (`nonce ?? null`, main.cjs:147);
the `BackendProcess` constructor destructures `nonce = null, requireLaunchNonce = false`
(main.cjs:343), so the optional fields match the real runtime shapes. `logger` was already declared.

### `studio/test/electronMain.test.ts` — narrow the indexed mock access

```diff
@@ describe('attachSecurityPolicy') — refuses <webview> attachment and every window open
     const handler = (win.webContents.setWindowOpenHandler as unknown as { mock: { calls: unknown[][] } })
-      .mock.calls[0][0] as () => { action: string };
+      .mock.calls[0]![0] as () => { action: string };
```

The preceding assertion already proves `setWindowOpenHandler` was called exactly once, so the
non-null assertion is sound and every assertion keeps its meaning.

### `studio/test/freecadVendorHashes.test.ts` — two strict-typing errors

```diff
@@ it('pins a reviewed SHA-256 digest list for every requirement')
-      const digests = pins.hashes[requirement];
+      const digests = pins.hashes[requirement]!;
       expect(digests, `hashes for ${requirement}`).toBeDefined();
```

The runtime `expect(...).toBeDefined()` check is kept; the assertion only satisfies
`noUncheckedIndexedAccess`, and a missing entry still fails before `.length` is read.

```diff
@@ it('refuses to invoke pip when the reviewed hashes are missing')
       vendor.downloadWheels({
         requirements,
-        hashes: undefined,
         pythonVersion: '3.12',
```

Under `exactOptionalPropertyTypes`, `hashes: undefined` is not assignable to `hashes?:`. Omitting the
property is the same call semantics and the test intent ("reviewed hashes are missing") is unchanged.

### `tests/unit/test_private_files.py` — stub accepts POSIX and Windows call shapes

```diff
@@ test_tighten_file_permissions_never_raises_for_an_unrestrictable_file
-    def refuse(_path) -> None:
+    def refuse(*_args, **_kwargs) -> None:
         raise OSError("icacls failed")
```

On POSIX `tighten_file_permissions` calls `os.chmod(path, mode)`; the stub replaces both
`restrict_windows_acl` and `os.chmod`, so it must tolerate one or two arguments. The stub now raises
`OSError` on both platforms. `src/kala_agent/private_files.py` was not touched.

## 2. Gates

All commands ran with a temporary `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR`,
`XDG_DATA_HOME` (`RUSTUP_HOME`/`CARGO_HOME`/`npm_config_cache` set as required); the real
`~/.local/share/kala-agent` was never touched.

| Gate | Command | Result |
|---|---|---|
| Typecheck | `cd studio && npm run typecheck` | **0 errors** (`tsc -p tsconfig.json --noEmit`, clean exit) |
| Studio tests | `cd studio && npm test` | **794 passed / 794**, 54 test files, 0 failed (3.43 s) |
| Python tests | `.venv/bin/python -m pytest -q -p no:cacheprovider tests/unit tests/security` | **1867 passed, 2 skipped, 0 failed** (61.01 s) |
| UI build | `cd studio && npm run build` | exit 0, Vite build succeeded |
| Package build | `packaging/linux/build-appimage.sh --no-smoke` | exit 0 |

The two skips are the Windows-only ACL tests in `test_private_files.py` (POSIX host), matching the
expected baseline: 1866 passed + the one formerly-failing test = **1867 passed, 2 skipped**.

## 3. Build and smoke

`packaging/linux/build-appimage.sh --no-smoke` ran the UI build, rebuilt the native release backend
from the current working tree (`cargo build --release --locked`; stage candidate replaced atomically,
`native-build.json` schema 3 and source parity regenerated), re-vendored the FreeCAD adapter for
cp311–cp314 (20 wheels, 668 files, FreeCAD 1.1.4 detected and in-FreeCAD import verified), copied
23 OCCT libraries with `RUNPATH=$ORIGIN/lib`, then electron-builder produced the AppImage.

The smoke was then run in its strict release form (the script's default run only reports geometry and
FreeCAD as PENDING, so the mandatory flags were passed explicitly):

```bash
node packaging/linux/smoke-appimage.mjs \
  --appimage studio/release/kala-studio-0.1.0-x86_64.AppImage \
  --require-geometry --require-freecad
```

**Verdict: PASS with FreeCAD provider** (`smoke_exit=0`). Key log lines:

```
[appimage-smoke] RPATH closure ok: 23 libTK resolutions from .../resources/backend/lib (empty environment, no LD_LIBRARY_PATH)
[appimage-smoke] FreeCAD Python deps ok: ABIs cp311, cp312, cp313, cp314 (20 wheels, 668 files; FreeCAD 1.1.4 / Python 3.14.8 (cp314))
[appimage-smoke] payload ok: metadata schema 3, 23 OCCT libs, FreeCAD adapter + 4 ABI sets, no Python runtime
[appimage-smoke] python-free backend env: PATH="" cwd=... (no repository/virtualenv path)
[appimage-smoke] health ok: {..."step_available":true,"design_available":true,...}
[appimage-smoke] packaged UI root ok
[appimage-smoke] unauthenticated refusal ok
[appimage-smoke] OpenCascade L-bracket ok: volume 35571.416047113315 mm3, measured_by OpenCascade (opt-in)
[appimage-smoke] drawing ok: front(7) top(11) right(8)
[appimage-smoke] section ok: XZ@4 area 2321.4601836602546 mm2 (exact), outer 1, inner 1
[appimage-smoke] STEP import ok: 55554 bytes, 1 solid(s), bounds {"min":[-30,0,0],"max":[30,45,40]}
[appimage-smoke] freecad backend env: PATH="/usr/bin:/bin:/usr/local/bin" cwd=... (no repository/virtualenv path)
[appimage-smoke] FreeCAD provider available: version 1.1.4
[appimage-smoke] FreeCAD L-bracket ok: volume 35571.416047 mm3, measured_by FreeCAD
[appimage-smoke] drawing ok: front(7) top(11) right(8)
[appimage-smoke] section ok: XZ@4 area 2321.4601836602546 mm2 (exact), outer 1, inner 1
[appimage-smoke] STEP import ok: 21648 bytes, 1 solid(s), bounds {"min":[-30,-8.881784197001e-16,0],"max":[30,45,40]}
[appimage-smoke] PASS (.../kala-studio-0.1.0-x86_64.AppImage) with FreeCAD provider
```

Expected values: volume `35571.416047` mm³ at rel 1e-6; section `2400 − 25π =
2321.460183660255…` mm² with tolerance 1e-9 and `area_exact=true`; STEP bounds within 0.001 mm.
Both phases match (the FreeCAD STEP `min.y` of −8.9e-16 is numeric noise, far inside 0.001 mm).

## 4. Artifact

```text
path:   /home/v9i/Projects/Kala-Agent/studio/release/kala-studio-0.1.0-x86_64.AppImage
size:   163682196 bytes
sha256: a35d377a3e9d8a997952ac690ecc0f357779135351c0bd0c398b37b392b2e7f4
```

Verified independently with `stat -c %s` and `sha256sum` after the smoke.

## 5. Notes / not fixed

- Nothing is left unfixed: all five reported blockers (three typecheck sites in `electronMain.test.ts`,
  two in `freecadVendorHashes.test.ts`, and the POSIX pytest failure) are resolved and re-verified.
- `main.cjs` and `src/kala_agent/private_files.py` are unchanged, per the brief.
- `packaging/linux/README.md` still records the *previous* artifact's size/SHA-256
  (163 889 259 / `bf4734e1…`); it was deliberately not edited because this cell's scope is the
  type/test fix plus packaging. The authoritative numbers for the new build are above.
- The build shell reused `ELECTRON_CACHE=/home/v9i/.cache/electron` and
  `ELECTRON_BUILDER_CACHE=/home/v9i/.cache/electron-builder` (tool caches only) while `HOME` itself
  stayed temporary; no kala data, credential or settings file was read or written.
- No git commits; the only tracked modifications are the four files listed in section 1.
- All temporary directories created for this cell (`/tmp/kala-fixpack-*`) were removed.
- `~/Applications/**` was not touched.
