# packaging — Rust-only desktop app, Linux AppImage, Windows build script

Early-start round (2026-10-20). `native-design` has **not landed**; the whole
pipeline was built against the current Rust `kala-studio`, and every smoke step
that does not need native geometry was executed. Geometry/FreeCAD smoke steps
are marked **PENDING native-design** below.

No commit, push, deploy, publish, signing, delegation, sudo, credential read or
Kala user-data access was performed. Every shell used the CONVENTIONS USER DATA
rule (temp `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR`,
`XDG_DATA_HOME`, and `RUSTUP_HOME`/`CARGO_HOME` as documented).

## Outcome

| Item | Result |
|---|---|
| Electron launches only the native runtime | Yes. Packaged mode resolves only `resources/backend/kala-studio[.exe]`; Python selection variables are removed from the child environment. Development now defaults to the staged native slice too. |
| OCCT shared-library staging | Implemented and exercised with a real ELF linked to `libTKernel`: `ldd` closure copied, `patchelf` set `$ORIGIN/lib`; staged binary ran with `LD_LIBRARY_PATH=<stage>/lib`. Current `kala-studio` links no OCCT yet, so the actual stage records 0 libs. |
| FreeCAD adapter staged | Yes: `adapters/freecad/src/kala_agent_freecad` (23 modules) + `adapters/freecad/mod/Kala` (3 files) = 26 files, with a tree digest. FreeCAD itself is not bundled. |
| Linux AppImage | Built: `studio/release/kala-studio-0.1.0-x86_64.AppImage`, 135,474,663 bytes (129.2 MiB), sha256 `29f3d82311c0f775011d0c68268652e5dc077cc098ad8c5beb862fb18dce9bba`. |
| AppImage smoke | PASS for payload/health/UI/auth/no-Python. L-bracket and FreeCAD provider **PENDING native-design**. |
| Windows script + README | Written; not executed (no Windows/pwsh on this host). |
| Release workflow | `.github/workflows/studio-release.yml` written, not pushed/run. |
| Old delivery REVIEW-2 objections | Independently re-probed; both are fixed. No further repair needed. |
| Studio gates | `npm run typecheck` exit 0; `npm test` 626/626 (43 files); `npm run build` exit 0. |

## Native-only Electron launch

`studio/electron/main.cjs`:

* Packaged mode resolves `resources/backend/kala-studio[.exe]` only. It never
  consults `KALA_STUDIO_PYTHON`, `KALA_STUDIO_DEV_BACKEND`, a PATH Python or a
  repository virtualenv.
* `packagedBackendEnvironment()` removes `PYTHONPATH`, `PYTHONHOME`,
  `KALA_STUDIO_PYTHON`, `KALA_STUDIO_DEV_BACKEND` and
  `KALA_STUDIO_GEOMETRY_WORKER` from the child environment. On Linux it
  prepends `resources/backend/lib` to `LD_LIBRARY_PATH`; on Windows it prepends
  the backend (and `lib` when present) to `PATH`. It also sets
  `KALA_FREECAD_SYS_PATH=resources/backend/adapters/freecad/src`.
* Development now defaults to `studio/backend-native/kala-studio`. The Python
  reference is available only behind an explicit `KALA_STUDIO_DEV_BACKEND=python`
  (or a `KALA_STUDIO_PYTHON` override). `scripts/desktop.cjs` stages the native
  slice by default and skips it only for that explicit Python mode.
* The release gate is the native-design readiness report
  (`desktop-pack.cjs` -> `native-readiness.cjs`), replacing the old
  `nativeStep` feature check. `desktop-pack` additionally refuses a stage that
  lacks the bundled FreeCAD adapter.

The AppImage smoke proves the packaged child starts with `PATH=""`, no Python
interpreter and a temp data directory.

## Staging (`studio/scripts/native-backend.cjs`, schema 3)

`studio/backend-native/` now contains:

```
kala-studio
studio_dist/
native-build.json
lib/                         optional, Linux when the binary links OCCT
adapters/freecad/src/kala_agent_freecad/
adapters/freecad/mod/Kala/
```

* **OCCT:** on Linux, `ldd` is run on the staged binary. Every resolved
  `libTK*.so*` is copied under its loader name, together with non-system
  dependencies outside `/lib*` and `/usr/lib*`. If `patchelf` exists, the
  binary gets `RUNPATH=$ORIGIN/lib` and each bundled library gets
  `RUNPATH=$ORIGIN`. The source Cargo binary is untouched. Without `patchelf`,
  the Electron `LD_LIBRARY_PATH` fallback is used; the smoke script mirrors it.
  `--occt-lib-dir` provides the explicit directory for Windows (vcpkg `bin`).
  Metadata records mode, each library's bytes/sha256, missing entries, `rpath`
  and the `patchelf` version.
* **FreeCAD:** the adapter and addon are copied with `__pycache__`, `.pyc`,
  `.pyo`, `.pyd` and `.egg-info` excluded. The adapter's tree digest is stored
  in `native-build.json` and re-verified before packaging. The
  `validateNativeTree` guard still rejects a Python interpreter, OCP,
  site-packages and vendor trees everywhere; only `.py` files below the
  explicit `adapters/freecad/**` bundle are allowed.
* **Staging safety:** the R3 transaction is preserved. Destination identity +
  state is captured before any awaited work, re-checked immediately before the
  swap, the moved backup is re-verified, and only a matching backup is removed.
  Optional `lib/`/`adapters/` and Windows top-level `.dll` entries are the only
  new allowed layout names.

`electron-builder.json` already copies the whole `backend-native` tree to
`resources/backend`; no change was needed there.

### FreeCAD adapter dependency caveat (honest residual)

The bundled adapter's Python imports are `kala_agent.contracts`,
`kala_agent.tools.protocol` and `pydantic`. Those are **not** vendored into the
AppImage. The adapter is found by the Rust provider (executable ancestors +
`KALA_FREECAD_SYS_PATH`), but `FreeCADCmd` must be able to import the adapter
dependencies — for example by installing `kala-agent-freecad` into FreeCAD's
Python. A per-FreeCAD-version dependency bundle is a separate deployment
decision; the provider fails honestly rather than fabricating a measurement.
This is called out in `packaging/linux/README.md` and
`packaging/windows/README.md`.

## Linux AppImage

Build command (also in `packaging/linux/build-appimage.sh`):

```bash
cd studio
npm run build
node scripts/native-backend.cjs
node scripts/desktop-pack.cjs --linux AppImage --x64 --publish never
```

Result:

| Field | Value |
|---|---|
| Path | `/home/v9i/Projects/Kala-Agent/studio/release/kala-studio-0.1.0-x86_64.AppImage` |
| Size | 135,474,663 bytes (129.2 MiB) |
| SHA-256 | `29f3d82311c0f775011d0c68268652e5dc077cc098ad8c5beb862fb18dce9bba` |
| Classification | `release` in `native-readiness.json`, with a warning that 10 route-methods still proxy to the Python geometry worker (pipeline build until native-design) |

`xvfb-run` is not installed on this host. The smoke therefore uses
`--appimage-extract` and runs `resources/backend/kala-studio` directly
(no FUSE, no X server), with `PATH=""`, temp `HOME`/`XDG_DATA_HOME`/
`KALA_AGENT_DATA_DIR`/`KALA_STUDIO_DATA_DIR`, an ephemeral token/port, and
`LD_LIBRARY_PATH=resources/backend/lib`.

Passed checks:

* extracted payload has `kala-studio`, `studio_dist`, `native-build.json`,
  the FreeCAD adapter and no Python runtime outside the adapter;
* metadata schema 3, adapter tree present, 0 current OCCT libs (the binary does
  not link OCCT yet), no missing closures;
* `/api/studio/health` -> `status: ok`;
* packaged UI root serves HTML;
* unauthenticated `POST /api/studio/preview` -> 401.

Pending checks (returned `503 geometry_worker_unavailable`, as expected):

* OpenCascade L-bracket `volume_mm3 = 35571.416047` (rel 1e-6), bounds
  `[-30,30] × [0,45] × [0,40]` — **PENDING native-design**;
* FreeCAD provider build — **PENDING native-design** (and the dependency
  caveat above).

Re-run after native-design:

```bash
node packaging/linux/smoke-appimage.mjs --require-geometry
node packaging/linux/smoke-appimage.mjs --require-freecad
```

A separate real-ELF probe (not the repository binary) linked against the
system `libTKernel` confirmed the new OCCT path end to end: one library copied
to `lib/`, `patchelf --print-rpath` returned `$ORIGIN/lib` for the binary and
`$ORIGIN` for the library, and the staged binary executed with exit 0.

## Windows

Files:

* `packaging/windows/build-windows.ps1` — prerequisite checks (MSVC rustup,
  Node 22.12+, git, vcpkg OCCT headers/`TKernel.dll`), npm ci, optional gates,
  Vite build, one `native-backend.cjs --occt-lib-dir <vcpkg bin>` call (builds
  `kala-studio.exe`, stages UI/adapter and copies the vcpkg DLLs next to the
  executable), `desktop-pack.cjs --win nsis --x64 --publish never`, artifact
  hash, and the post-install smoke checklist.
* `packaging/windows/README.md` — prerequisites, exact commands/switches,
  package layout, freecad caveat, smoke checklist, signing/publishing limits.

Not executed here: this host has no Windows, no `pwsh`, and no
`x86_64-w64-mingw32-gcc`. `rustup target list --installed` does include
`x86_64-pc-windows-gnu`, but the MinGW compiler is absent, so the conditional
non-OCCT windows-gnu cross-check was not run; that remains with the
`windows-check` cell. The script is a definition, not Windows-runtime proof.

## Release workflow

`.github/workflows/studio-release.yml` (written only; not pushed or run):

* `workflow_dispatch` and `v*` tags; `permissions: contents: read`;
* Linux job: Node 22 + Rust, `patchelf`, vcpkg `opencascade:x64-linux`, Studio
  gates, UI/native stage, AppImage, headless smoke with `--require-geometry`,
  sha256 + artifact upload;
* Windows job: Node 22 + stable MSVC + NASM, vcpkg `opencascade:x64-windows`,
  `build-windows.ps1 -RunGates`, NSIS, sha256 + artifact upload;
* no signing, publishing, deploy or upload API use.

## Old delivery REVIEW-2 objections — verified

Both were re-probed independently against the current helpers and tests; no
repair was necessary.

1. **Concurrent destination creation during async staging.**
   `stageNativeBackend` with a nonexistent temp destination and an
   `onBeforeInstall` hook that creates the destination plus `user-sentinel.txt`
   was refused with:
   `refusing to stage: … appeared while the replacement was being built. Any
   concurrent directory and its files were left untouched.`
   The sentinel survived byte-for-byte and no `.staging-*`/`.backup-*`
   leftovers remained. The deterministic regression tests are in
   `studio/test/nativeBackendPackaging.test.ts`
   (`refuses a destination created while the replacement is being built…`,
   plus the replaced/modified/backup-preservation cases).
2. **Negated `--dir` release-gate bypass.**
   The installed electron-builder parser normalizes
   `['--dir','--dir=false','--win','nsis']`,
   `['--dir','--no-dir','--win','nsis']` and `['--dir','--win','nsis']` to
   effective `windows:nsis`; `decidePackaging` blocks all three while
   readiness is pending. The installed-parser parity and negated/conflicting
   matrices remain in the test file, and publication is still forced to
   `never`.

The unreviewed R3 result therefore holds for both objections; no accepted
source, test or parity result was weakened.

## Commands run

| Check | Result |
|---|---|
| `npm run typecheck` | exit 0 |
| `npm test` | 626/626 passed, 43 files |
| `npx vitest run test/nativeBackendPackaging.test.ts test/electronMain.test.ts test/electronBootstrap.test.ts` | 83/83 passed |
| `npm run build` | exit 0 |
| `node scripts/native-backend.cjs` | exit 0; staged 26 adapter files, 0 current OCCT libs |
| `node scripts/desktop-pack.cjs --linux AppImage --x64 --publish never` | exit 0; AppImage built |
| `node packaging/linux/smoke-appimage.mjs --appimage …` | PASS; geometry/FreeCAD PENDING native-design |
| fake `libTKernel` ELF stage probe | copied 1 lib, `$ORIGIN/lib`/`$ORIGIN` patched, staged binary exit 0 |
| REVIEW-2 race probe | refused; sentinel preserved; no leftovers |
| REVIEW-2 builder-gate probe | all three negated/conflicting forms blocked as `windows:nsis` |
| `node scripts/native-readiness.cjs --report-only` | exit 0; 15 Python route-methods, 6 Rust-native + 10 proxied, ready |
| `node -c`/`bash -n` on new packaging scripts | exit 0 |
| YAML parse of the workflow (`js-yaml`) | valid; jobs `linux-appimage`, `windows-nsis` |

## Changed paths

* `studio/electron/main.cjs`, `studio/electron/main.d.cts` — native-only
  packaged environment, native dev default, explicit Python fallback.
* `studio/scripts/native-backend.cjs`, `studio/scripts/native-backend.d.cts` —
  schema 3 staging, OCCT `ldd`/patchelf closure, FreeCAD adapter bundle,
  optional layout entries, adapter/library digest verification.
* `studio/scripts/desktop-pack.cjs` — native-design readiness gate (old
  `nativeStep` check removed), bundled-adapter requirement, pipeline-build
  warning while the worker is still proxied.
* `studio/scripts/desktop.cjs` — native dev default; Python only behind an
  explicit reference mode.
* `studio/test/electronMain.test.ts`,
  `studio/test/nativeBackendPackaging.test.ts` — packaged env sanitization,
  native default, adapter/library staging, REVIEW-2 regressions.
* NEW `packaging/linux/{build-appimage.sh,smoke-appimage.mjs,README.md}`.
* NEW `packaging/windows/{build-windows.ps1,README.md}`.
* NEW `.github/workflows/studio-release.yml` (not pushed).
* This report.

`studio/electron-builder.json` was verified (it already copies
`backend-native` -> `resources/backend`) and left unchanged. Pre-existing
uncommitted changes from other waves/cells were read-only and untouched.

## Remaining gaps

1. **Native-design geometry** is pending. The AppImage is a pipeline build
   until the L-bracket and FreeCAD smokes pass with `--require-geometry` /
   `--require-freecad`.
2. **FreeCAD adapter Python dependencies** are not vendored; FreeCAD's
   interpreter must provide `kala_agent.contracts`, `kala_agent.tools.protocol`
   and `pydantic`. A per-FreeCAD-version dependency bundle is not implemented.
3. **Windows runtime is unexecuted.** The script and workflow are definitions;
   the user's Windows machine must run the post-install checklist. MinGW is
   absent here, so the conditional windows-gnu cross-check was not run.
4. **`xvfb-run` is absent** (patchelf 0.19.2 is present). The documented
   extraction path is used; Electron window rendering was not exercised.
5. The AppImage is unsigned and not published; the release workflow was not
   run. Solid Edge remains Windows-only and unreachable from Linux.
6. The current `kala-studio` links no OCCT, so the real AppImage has no
   `lib/` yet. The OCCT staging code path was validated with a real
   `libTKernel`-linked ELF probe; native-design's build must be re-staged to
   copy the actual `libTK*.so` closure.
7. Pre-existing files outside this cell's ownership still describe the prior
   Python-default development flow or the old `nativeStep` marker:
   `studio/README.md`, `.github/workflows/studio.yml` and
   `packaging/native_studio_smoke.py`. They were not edited. The new
   `packaging/linux/smoke-appimage.mjs` verifies the packaged runtime without
   the `nativeStep` requirement.

---

# Round 2 — native geometry + vendored FreeCAD interpreter (2026-10-20)

Status: **complete**. `native-design` and `cad-r3` landed; the AppImage was
rebuilt with the OCCT-linked native `kala-studio`, the real `libTK*` closure is
staged, every previously pending smoke step was executed from the extracted
AppImage (whole smoke under `env -i`), and the FreeCAD adapter's Python
dependencies are now vendored for FreeCAD's own interpreter and proven by a
real FreeCAD provider build from the extracted app.

No commit, push, deploy, publish, signing, delegation, sudo or credential read.
Every shell used the USER DATA rule (`RUSTUP_HOME=/home/v9i/.rustup`,
`CARGO_HOME=/home/v9i/.cargo`, temp `HOME`, `KALA_AGENT_DATA_DIR`,
`KALA_STUDIO_DATA_DIR`, `XDG_DATA_HOME`); every backend run used an explicit
temp `--data-dir`/environment. `~/.local/share/kala-agent/**` was never named,
read or written.

## 1. Final AppImage

| Field | Value |
|---|---|
| Path | `/home/v9i/Projects/Kala-Agent/studio/release/kala-studio-0.1.0-x86_64.AppImage` |
| Size | **157,668,547 bytes (150.4 MiB)** |
| SHA-256 | **`ecbb1c538eee740771b435241db7dc3a83ef7b132f190bda6d6633a5bed15447`** |
| Classification | `release` / `releaseEligible: true` in `release/native-readiness.json` |

Build chain (all exit 0): `npm run build` → `node scripts/native-backend.cjs
--require-freecad-vendor` → `node scripts/desktop-pack.cjs --linux AppImage
--x64 --publish never`.

## 2. Native stage: real OCCT closure + FreeCAD vendor

`studio/backend-native/` (schema 3):

| Item | Result |
|---|---|
| `kala-studio` | 11,217,505 bytes, sha256 `064993786af252f276d1823c6aef70d54b32daa1b5df46fa92e4354edcdceaab`; links OpenCascade 7.9.3 |
| OCCT closure | **23** `libTK*.so*` files copied from the exact `ldd` resolution into `lib/` |
| Relocation | `patchelf 0.19.2`: binary `RUNPATH=$ORIGIN/lib`, each library `RUNPATH=$ORIGIN`. The Cargo binary was not modified. |
| `native-build.json` | records mode `ldd`, each library's bytes/sha256, missing list (empty), `rpath`, `patchelf`, binary/UI digests, source parity, FreeCAD vendor manifest |
| FreeCAD adapter | 264 files total (`src/kala_agent_freecad` + `mod/Kala` + vendored deps + `vendor.json`) |
| Python runtime | none outside `adapters/freecad/src`; no `python`/`libpython`/`.pyc`/`__pycache__` anywhere |

The extracted runtime was proved to resolve all 23 `libTK*` from
`resources/backend/lib` **with an empty environment and no
`LD_LIBRARY_PATH`** (`ldd` under `env: {}`), not from the host's
`/usr/lib`. The smoke therefore omits `LD_LIBRARY_PATH` when the RPATH is
present.

## 3. Pending smoke steps — all executed from the extracted AppImage

The complete smoke was run under a literal `env -i`:

```sh
env -i PATH=/usr/bin:/bin HOME=<tmp> TMPDIR=<tmp> XDG_DATA_HOME=<tmp> \
  KALA_AGENT_DATA_DIR=<tmp> KALA_STUDIO_DATA_DIR=<tmp> \
  /home/v9i/.local/share/mise/installs/node/26.8.2/bin/node \
  packaging/linux/smoke-appimage.mjs --require-geometry --require-freecad
```

Exit 0. Two explicit child environments, nothing inherited:

**Phase 1 — `PATH=""` (no Python on PATH at all):**

| Check | Result |
|---|---|
| payload | schema 3, 23 OCCT libs, adapter + 5 wheels, no Python runtime |
| `/api/studio/health` | `status: ok`, `design_available: true`, `step_available: true`, `geometry_worker: {state: ready, version: 7.9.3}` |
| packaged UI | `GET /` serves HTML |
| auth | unauthenticated `POST /api/studio/preview` → 401 |
| OpenCascade L-bracket | **35571.416047113315 mm³**, bounds `[-30,0,0] … [30,45,40]`, `measured_by: OpenCascade (opt-in)` |
| drawing | `front(7) top(11) right(8)` visible segments |
| section | XZ at `y=4`: **2321.4601836602546 mm² = 2400 − 25π**, `area_exact: true`, 1 outer + 1 inner loop |
| STEP round trip | export 55,554 bytes; import returns 1 solid, same digest, bounds equal within 0.001 mm |

**Phase 2 — `PATH=/usr/bin:/bin:/usr/local/bin`, cwd in a temp dir, no
`PYTHONPATH`, no `PYTHONHOME`, no `KALA_FREECAD_SYS_PATH`, no repository and
no virtualenv in any env value:**

| Check | Result |
|---|---|
| FreeCAD phase health | ok |
| provider list | `freecad` available, version `1.1.4` |
| FreeCAD L-bracket | **35571.416047 mm³**, bounds `[-30,-8.88e-16,0] … [30,45,40]`, `measured_by: FreeCAD` |
| drawing | `front(7) top(11) right(8)` |
| section | same exact area 2321.4601836602546 mm², 1/1 loops |
| STEP round trip | export 21,648 bytes; import 1 solid, bounds equal within 0.001 mm |

## 4. FreeCAD Python dependencies vendored for FreeCAD's interpreter

Residual from round 1 is closed.

Detection (`studio/scripts/freecad-python-deps.json` pins, helper
`studio/scripts/freecad-python-vendor.cjs`):

* `FreeCADCmd` at `/usr/bin/FreeCADCmd`, **FreeCAD 1.1.4 Revision 45040**;
* its interpreter: **Python 3.14.8**, cache tag `cpython-314`, ABI **cp314**,
  SOABI `cpython-314-x86_64-linux-gnu`, platform `linux-x86_64`;
* wheels resolved with `pip download --only-binary=:all: --python-version 3.14
  --implementation cp --abi cp314 --platform manylinux2014_x86_64`.

Staged into `adapters/freecad/src` (the exact directory the native provider
hands to `FreeCADCmd` as its `sys.path` root), so no Rust or adapter change was
needed:

| Wheel | Bytes | SHA-256 |
|---|---|---|
| `annotated_types-0.8.0-py3-none-any.whl` | 13,427 | `f072f4d804ea359e4eaf198b1af7a8b0943881a87f31bb764f8bf219bb9419e0` |
| `pydantic-2.13.5-py3-none-any.whl` | 472,589 | `346a034f080da3755d8e9cb5e00e8b07de1d39e4f6e2c87d8ab7cafa0b269a73` |
| `pydantic_core-2.46.5-cp314-cp314-manylinux_2_17_x86_64.manylinux2014_x86_64.whl` | 2,068,352 | `54d510bac3ee52247af28ed4bb18a1e799f040ac60fd2bf5ccd4c92f1fbe786f` |
| `typing_extensions-4.16.0-py3-none-any.whl` | 45,571 | `481caa481374e813c1b176ada14e97f1f67a4539ce9cfeb3f350d78d6370c2e8` |
| `typing_inspection-0.4.4-py3-none-any.whl` | 14,750 | `65b8397ba37ccbce054456aaccddfc91e6e3083c92824df348d96ca832f3f147` |

Also vendored: `kala_agent` (95 `.py` files, tree sha256
`bbe4997eb9e71b361e8b3f70fbe5aceeaa5128c5e8cf2ed362049338390a168f`) because
the adapter imports `kala_agent.contracts`, `kala_agent.contracts.construction`
and `kala_agent.tools.protocol`, and importing a submodule executes the package
`__init__`. Total vendored tree: 260 files, 11 top-level roots, recorded in
`adapters/freecad/vendor.json` and mirrored into
`native-build.json.freeCadAdapter.vendor`.

Build-time verification: the helper imports `pydantic`,
`kala_agent.contracts`, `kala_agent.tools.protocol` and
`kala_agent_freecad.worker` **inside `/usr/bin/FreeCADCmd`** before packaging;
recorded result `{"pydantic":"2.13.5","protocol":"1.0"}`. FreeCAD's embedded
interpreter ignored `PYTHONDONTWRITEBYTECODE` and wrote 14 bytecode entries
during that probe; they were pruned before hashing (0 remain in the stage).

Direct provenance proof from an extracted AppImage, `env -i`, FreeCAD's own
interpreter:

```json
{"pydantic": ".../squashfs-root/resources/backend/adapters/freecad/src/pydantic/__init__.py",
 "pydantic_version": "2.13.5",
 "pydantic_core": ".../squashfs-root/resources/backend/adapters/freecad/src/pydantic_core/__init__.py",
 "kala_agent": ".../squashfs-root/resources/backend/adapters/freecad/src/kala_agent/__init__.py",
 "kala_agent_freecad": ".../squashfs-root/resources/backend/adapters/freecad/src/kala_agent_freecad/__init__.py",
 "python": "3.14.8", "executable": "/usr/bin/FreeCADCmd"}
```

No path outside the extraction (and no `.venv`/repository path) appears.

Guard changes: `native-backend.cjs` now accepts `.py`/`.so`/`.pyd` only inside
the explicit FreeCAD adapter + vendor roots; interpreter binaries, `libpython`,
`site-packages`, `__pycache__`/`.pyc` and `.egg-info` remain refused.
`desktop-pack.cjs` refuses to package a stage without `vendor.json`.

## 5. Windows equivalent (`build-windows.ps1`)

`packaging/windows/build-windows.ps1` now:

1. locates `FreeCADCmd.exe` (`-FreeCadExe`, then `PATH`, then
   `%ProgramFiles%\FreeCAD*\bin`);
2. runs a probe script in that interpreter and reads `sys.version_info`,
   `cache_tag` and `sysconfig.get_platform()` (documented in the file header);
3. passes `--freecad-python <FreeCADCmd> --require-freecad-vendor` (or
   `--freecad-python-version/--freecad-abi` when FreeCAD is absent, with a
   warning);
4. clears `PYTHONPATH`/`PYTHONHOME` before staging;
5. verifies after staging that `vendor.json`'s ABI matches the detected
   interpreter, that `kala_agent`/`pydantic`/`pydantic_core` and
   `_pydantic_core*.pyd` exist, and that the OCCT `TK*.dll` closure was copied;
6. prints the detected FreeCAD Python and wheel count, and the README/checklist
   state that no virtualenv/repository is needed on the target.

`packaging/windows/README.md` and `packaging/linux/README.md` were updated
accordingly, and `.github/workflows/studio-release.yml` now logs the vendor
manifest and runs `--require-freecad` only when FreeCAD is installed on the
runner (still written only, not pushed or run).

**Not executed:** this host has no Windows and no `pwsh`, so the PowerShell is
reviewed but not executed. The Windows runtime remains the user's checklist.

## 6. Gates and commands (Round 2)

| Check | Result |
|---|---|
| `npm run typecheck` | exit 0 |
| `npm test` | **627/627 passed, 43 files** |
| `npx vitest run test/nativeBackendPackaging.test.ts test/electronMain.test.ts test/electronBootstrap.test.ts` | **84/84 passed** |
| `npm run build` | exit 0 |
| `node scripts/native-backend.cjs --require-freecad-vendor` | exit 0; 23 OCCT libs, `$ORIGIN/lib`, 5 wheels / 260 vendor files |
| `node scripts/desktop-pack.cjs --linux AppImage --x64 --publish never` | exit 0; AppImage built |
| `env -i … smoke-appimage.mjs --require-geometry --require-freecad` | **PASS**, all tables in §3 |
| `bash -n packaging/linux/build-appimage.sh` | exit 0 |
| YAML parse of `studio-release.yml` | valid; jobs `linux-appimage`, `windows-nsis` |
| `patchelf --version` | `patchelf 0.19.2` |
| `xvfb-run` | absent; documented extraction path used |

## 7. Changed paths (Round 2)

* NEW `studio/scripts/freecad-python-deps.json` — pins + interpreter defaults.
* NEW `studio/scripts/freecad-python-vendor.cjs` — detection, `pip download`,
  wheel unpacking, `kala_agent` vendoring, in-FreeCAD import verification,
  manifest + digests.
* `studio/scripts/native-backend.cjs` (+ `.d.cts`) — vendor integration,
  vendor-aware forbidden-name rules, metadata, CLI flags
  (`--no-vendor-freecad`, `--require-freecad-vendor`, `--freecad-python`,
  `--freecad-python-version`, `--freecad-abi`, `--vendor-python`).
* `studio/scripts/desktop-pack.cjs` — refuses a stage without the FreeCAD
  vendor; accurate geometry-seam warning text.
* `studio/scripts/native-readiness.cjs` — summary wording only (counts and
  readiness logic unchanged).
* `studio/test/nativeBackendPackaging.test.ts` — new vendor-tree accept/refuse
  test.
* `packaging/linux/smoke-appimage.mjs` — two-phase smoke, RPATH `ldd` closure
  check, drawing/section/STEP round trip, vendor checks, no-repo/venv env
  assertion, `--freecad-path`.
* `packaging/linux/build-appimage.sh`, `packaging/linux/README.md` — FreeCAD
  interpreter selection (`KALA_FREECAD_PYTHON`,
  `KALA_REQUIRE_FREECAD_VENDOR`), vendoring and smoke documentation.
* `packaging/windows/build-windows.ps1`, `packaging/windows/README.md` —
  Windows FreeCAD interpreter detection, vendoring, post-stage checks,
  updated package layout and checklist.
* `.github/workflows/studio-release.yml` — vendor log + conditional FreeCAD
  smoke (not pushed, not run).

## 8. Remaining gaps (Round 2)

1. **The vendored wheel set is ABI-coupled to the build host's FreeCAD.**
   Here: FreeCAD 1.1.4 / Python 3.14.8 / `cp314`. The official FreeCAD 1.1.4
   Linux AppImage ships Python 3.11 (`py311`), so this AppImage's
   `pydantic_core` would not load under that FreeCAD. Rebuild with
   `KALA_FREECAD_PYTHON=<that FreeCADCmd>` (Linux) or `-FreeCadExe` (Windows)
   to target another interpreter. Documented in both READMEs.
2. **Windows runtime is unexecuted** (no Windows/pwsh here). The script,
   detection and checks are definitions; the user's post-install checklist is
   the proof.
3. **`xvfb-run` is absent**, so Electron window rendering is still not
   exercised; the packaged backend/UI/API are verified by extraction.
4. **The AppImage is unsigned and not published**; the release workflow was
   not run. Solid Edge remains Windows-only.
5. **FreeCAD itself is not bundled**; the provider correctly reports
   unavailable when `FreeCADCmd` is absent.
6. Pre-existing files outside this cell still describe the old flow
   (`studio/README.md`, `.github/workflows/studio.yml`,
   `packaging/native_studio_smoke.py`); they were not edited.

