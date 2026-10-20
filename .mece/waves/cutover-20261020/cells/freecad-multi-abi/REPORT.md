# freecad-multi-abi — one bundled FreeCAD adapter for cp311–cp314

Status: **complete**. The adapter's Python dependencies are now vendored for
every CPython ABI current FreeCAD releases ship (cp311, cp312, cp313, cp314) on
Linux `manylinux2014_x86_64` and Windows `win_amd64`; the Rust provider probes
`FreeCADCmd`'s own Python (cached) and puts exactly the matching
`site-packages-cpNNN` directory on `sys.path`; an unsupported ABI is an honest
provider-unavailable reason naming the version. The AppImage was rebuilt and
the extracted artifact builds the FreeCAD L-bracket at **35571.416047 mm³** on
this host.

No commit, push, deploy, publish, signing, delegation, sudo, credential read or
Kala user-data access was performed. Every shell used the CONVENTIONS USER DATA
rule (`RUSTUP_HOME=/home/v9i/.rustup`, `CARGO_HOME=/home/v9i/.cargo`, temp
`HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR`, `XDG_DATA_HOME`), and
every backend run used a temp `--data-dir`/environment. `~/.local/share/kala-agent/**`
was never named, read or written. Python 3.11–3.13 came from `uv` with
`UV_PYTHON_INSTALL_DIR=/tmp/opencode/uv-python`; pip caches were under
`/tmp/opencode`.

## 1. Final Linux AppImage

| Field | Value |
|---|---|
| Path | `/home/v9i/Projects/Kala-Agent/studio/release/kala-studio-0.1.0-x86_64.AppImage` |
| Size | **163,642,516 bytes (156.1 MiB)** |
| SHA-256 | **`b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e`** |
| Classification | `release`, `releaseEligible: true` in `studio/release/native-readiness.json` |
| Native binary | 11,242,113 bytes, sha256 `fd754709cfbde6c89fce75b03a960bf346001fa30c36d4857dcda8d6c6b4e167` |
| OCCT closure | 23 `libTK*` libraries from `ldd`, `RUNPATH=$ORIGIN/lib` |
| FreeCAD adapter | 690 files total; 4 ABI sets × 142 files + `src/kala_agent` 95 files + adapter/addon |

Build chain (all exit 0): `npm run build` → `node scripts/native-backend.cjs
--require-freecad-vendor` → `node scripts/desktop-pack.cjs --linux AppImage
--x64 --publish never`.

## 2. Vendoring layout and `vendor.json`

Staged layout under `<stage>/adapters/freecad/`:

```
vendor.json                    schema 2: ABIs, per-ABI wheel digests, detection
src/kala_agent_freecad/        adapter modules
src/kala_agent/                shared pure-Python dependency (95 files)
site-packages-cp311/           full wheel set for Python 3.11 (142 files)
site-packages-cp312/           full wheel set for Python 3.12 (142 files)
site-packages-cp313/           full wheel set for Python 3.13 (142 files)
site-packages-cp314/           full wheel set for Python 3.14 (142 files)
mod/Kala/                      FreeCAD workbench addon
```

Each ABI directory contains the same five pinned wheels:

| Wheel | ABI-specific |
|---|---|
| `pydantic-2.13.5-py3-none-any.whl` | no |
| `pydantic_core-2.46.5-cpNNN-cpNNN-<platform>.whl` | **yes** |
| `annotated_types-0.8.0-py3-none-any.whl` | no |
| `typing_extensions-4.16.0-py3-none-any.whl` | no |
| `typing_inspection-0.4.4-py3-none-any.whl` | no |

The wheels are resolved with `pip download --only-binary=:all:` per ABI, so a
pure-Python wheel can never silently substitute for `pydantic_core`. The staged
manifest records, per ABI: Python version, wheel tag, every wheel name/bytes/
SHA-256, package roots, file count and tree digest; it also records the detected
FreeCAD build, the in-FreeCAD import verification result, the shared
`kala_agent` tree digest and the total adapter dependency-tree digest. The
release gate (`desktop-pack.cjs` → `verifyStageIntegrity` →
`validateFreeCadVendorManifest`) refuses a stage that lacks any of the four ABI
sets, a `pydantic_core` C extension, a wheel digest, or a package root.

The new repository-level `adapters/freecad/vendor.json` is a
`kind: dependency-matrix` document with **40 wheel digests** resolved for both
`linux-x86_64`/`manylinux2014_x86_64` and `win_amd64`:

```
node studio/scripts/freecad-python-vendor.cjs --matrix
```

Wheel digests in the staged manifest and the matrix are identical for the
Linux target (same pinned wheel files). The staged manifest remains the
authoritative packaged record; the matrix is the cross-platform proof and the
documented source of the `win_amd64` digests (regenerate it if the pins change).

## 3. Runtime ABI selection (Rust)

`backend/crates/kala-cad-providers/src/freecad.rs`:

* New public API (documented, exported from `lib.rs`):
  `BundledPythonSelection` (`Unbundled` / `Matched { abi, directory }` /
  `Unsupported { detected, supported }`), `bundled_python_abis(adapter_src)`,
  `select_bundled_python_abi(adapter_src, abi)`, `SITE_PACKAGES_PREFIX`,
  `SUPPORTED_FREECAD_PYTHON_ABIS`, `PYTHON_PROBE_FILENAME`.
* When discovery (or an explicit `sys_path`) points at an adapter `src` whose
  parent contains `site-packages-cpNNN`, `probe()` runs a small script in
  `FreeCADCmd` (`sys.version.split()`, `sys.implementation.cache_tag`), derives
  the ABI (`cpython-314` → `cp314`) and caches the selection with the existing
  availability probe (`DEFAULT_PROBE_CACHE_SECONDS`, default 30 s).
* `driver_sys_path()` adds **only** the matching directory, after `src`:
  `KALA_FREECAD_SYS_PATH=<src>:<site-packages-cpNNN>`. No other ABI directory
  is ever placed on the path.
* An unsupported ABI returns `available: false` with e.g.
  `FreeCADCmd runs Python 3.14.8 (cp314); the bundled adapter dependencies
  support cp310, cp311, cp312, cp313 only. Install a FreeCAD build with a
  supported Python.` No panic, no partial install, no fallback to another ABI's
  compiled extension. A probe failure is likewise an unavailable reason.
* A development checkout with no `site-packages-*` directory keeps the old
  single-path behaviour and spawns no extra process, so the existing repository
  differential is unchanged.

No Rust public behaviour changed for the no-bundle case; the differential,
Solid Edge tests and contract tests are unchanged and green.

## 4. Proof

### 4.1 End-to-end parity from the extracted AppImage

The complete smoke ran under a literal `env -i`:

```sh
env -i PATH=/usr/bin:/bin HOME=<tmp> TMPDIR=<tmp> XDG_DATA_HOME=<tmp> \
  KALA_AGENT_DATA_DIR=<tmp> KALA_STUDIO_DATA_DIR=<tmp> \
  /home/v9i/.local/share/mise/installs/node/26.8.2/bin/node \
  packaging/linux/smoke-appimage.mjs --require-geometry --require-freecad
```

Result: **PASS** for the final artifact
`kala-studio-0.1.0-x86_64.AppImage` (sha256 above).

| Check | Result |
|---|---|
| payload | metadata schema 3, 23 OCCT libs, FreeCAD adapter + **4 ABI sets**, no Python runtime |
| RPATH closure | 23 `libTK*` resolved from `resources/backend/lib` with an empty environment |
| `/api/studio/health` | `status: ok`, `design_available: true`, OCCT 7.9.3 ready |
| packaged UI / auth | HTML root served; unauthenticated preview → 401 |
| OpenCascade L-bracket | **35571.416047113315 mm³**, bounds `[-30,0,0]…[30,45,40]` |
| drawing / section | `front(7) top(11) right(8)`; XZ@4 = 2321.4601836602546 mm² = 2400−25π exact |
| STEP round trip | 55,554 bytes, 1 solid, bounds equal |
| FreeCAD L-bracket | **35571.416047 mm³**, `measured_by: FreeCAD`, bounds `[-30,-8.88e-16,0]…[30,45,40]` |
| FreeCAD drawing / section / STEP | same counts; same exact section; 21,648 bytes, 1 solid |

The FreeCAD phase used the host `/usr/bin/FreeCADCmd` (FreeCAD 1.1.4, Python
3.14.8, `cp314`), the extracted app's `site-packages-cp314`, no repository, no
virtualenv, no `PYTHONPATH`, and no `KALA_FREECAD_SYS_PATH` in the child
environment. Direct provenance from the extracted app:

```json
{"python":"3.14.8","cacheTag":"cpython-314","pydantic":"2.13.5",
 "pydantic_path":".../site-packages-cp314/pydantic/__init__.py",
 "core_path":".../site-packages-cp314/pydantic_core/__init__.py",
 "adapter":".../src/kala_agent_freecad/__init__.py"}
```

### 4.2 Selection logic, cp311–cp314

Rust unit tests (fake `FreeCADCmd`, `tests/freecad_unit.rs`):

* `every_supported_abi_selects_only_its_own_directory` — `bundled_python_abis`
  returns exactly `cp311, cp312, cp313, cp314`; `select_bundled_python_abi`
  returns the matching `site-packages-cpNNN` for each; `cp310` returns
  `Unsupported`; a directory with no ABI sets returns `Unbundled`.
* `the_build_puts_only_the_matching_abi_directory_on_sys_path` — for each of
  cp311–cp314 the fake provider receives exactly two `KALA_FREECAD_SYS_PATH`
  entries, `<src>` and `<site-packages-cpNNN>`, never another ABI.
* `an_unsupported_python_reports_an_honest_unavailable_reason` — a fake Python
  3.10.14/`cp310` yields `available: false`; the reason names `3.10.14`,
  `cp310` and all four supported ABIs; `build()` returns the same unavailable
  reason and does not crash.

### 4.3 Every ABI directory imports with a matching interpreter

The final AppImage was extracted and each `site-packages-cpNNN` + `src` was
imported with an interpreter of that ABI (adapter, `kala_agent.contracts`,
`kala_agent.tools.protocol`, `pydantic`, `pydantic_core`):

| ABI | Interpreter | Result |
|---|---|---|
| cp311 | `uv` CPython **3.11.16** | pydantic 2.13.5, pydantic_core 2.46.5, protocol 1.0 |
| cp312 | `uv` CPython **3.12.14** | same |
| cp313 | `uv` CPython **3.13.15** | same |
| cp314 | system CPython **3.14.8** (`/usr/bin/python3.14`) | same |

`FreeCADCmd` (the actual 3.14.8 embedded interpreter) also imported the
extracted `site-packages-cp314` + `src` with the same versions and paths.

### 4.4 Unsupported ABI from the final extracted AppImage

In a throwaway extraction, `site-packages-cp314` was renamed to
`site-packages-cp310` and the packaged `kala-studio` was started with the host
FreeCAD 3.14.8. `/api/studio/providers/cad` returned:

```
freecad: available=false
reason: FreeCADCmd runs Python 3.14.8 (cp314); the bundled adapter dependencies
        support cp310, cp311, cp312, cp313 only. Install a FreeCAD build with
        a supported Python.
```

The directory was restored; the real AppImage itself was never modified.

## 5. Tests and gates

| Check | Result |
|---|---|
| `cargo fmt --check` | exit 0 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | exit 0 |
| `cargo test --locked` (debug) | 40/40: 23 freecad_unit + 1 freecad_differential + 3 solidedge_differential + 13 solidedge_mock_bridge |
| `cargo test --locked --release` | 40/40, same suites |
| FreeCAD live differential | **43 builds, 0 refusals**, 28 corpus + 15 randomized; 0 mismatches |
| `npm run typecheck` | exit 0 |
| `npm test` | **628/628 passed, 43 files** |
| `npm run build` | exit 0 |
| `node scripts/native-backend.cjs --require-freecad-vendor` | exit 0; 4 ABIs, 20 wheels, 663 dependency files, FreeCAD verified |
| `node scripts/desktop-pack.cjs --linux AppImage --x64 --publish never` | exit 0 |
| `env -i … smoke-appimage.mjs --require-geometry --require-freecad` | **PASS** |
| `node scripts/freecad-python-vendor.cjs --matrix` | exit 0; 40 wheel digests (Linux + Windows) |
| `node --check` / `bash -n` on changed scripts | exit 0 |

Studio tests added/updated in `studio/test/nativeBackendPackaging.test.ts`
(45 tests in that file): the four-ABI vendor tree is accepted; `cp310` is not a
recognised ABI directory; `src/kala_agent` is the shared vendor root; a missing
`site-packages-cp313` is refused with its path; interpreter/`site-packages`
payloads remain refused; and the checked matrix has both platforms, all four
ABIs, a cpNNN `pydantic_core` wheel, and 64-hex digests.

## 6. Changed paths

* `studio/scripts/freecad-python-deps.json` — schema 2 pins with
  `python.supported` = cp311/cp312/cp313/cp314.
* `studio/scripts/freecad-python-vendor.cjs` — per-ABI wheel resolution and
  unpacking, shared `kala_agent`, per-ABI verification, schema-2 manifest,
  bytecode pruning, `--matrix` cross-platform digest document.
* `studio/scripts/native-backend.cjs` + `native-backend.d.cts` — ABI directory
  recognition/validation, schema-2 vendor checks, per-ABI metadata, updated
  CLI/usage.
* `studio/test/nativeBackendPackaging.test.ts` — four-ABI staging/vendor tests
  and the matrix test.
* `packaging/linux/smoke-appimage.mjs` — schema-2/4-ABI vendor verification;
  log wording.
* `packaging/linux/build-appimage.sh` — FreeCAD env-var documentation.
* `packaging/linux/README.md` — multi-ABI layout and runtime selection.
* `packaging/windows/build-windows.ps1` — always vendor all four `win_amd64`
  ABIs; detection now only verifies; post-stage checks per ABI; updated
  checklist.
* `packaging/windows/README.md` — multi-ABI layout, switches, checklist.
* `adapters/freecad/vendor.json` — NEW `dependency-matrix` with 40 wheel
  digests for Linux and Windows.
* `backend/crates/kala-cad-providers/src/freecad.rs` — ABI discovery/selection,
  cached Python probe, honest unsupported reason, single matching path.
* `backend/crates/kala-cad-providers/src/lib.rs` — new public exports.
* `backend/crates/kala-cad-providers/tests/fixtures/fake_freecad.py` — answers
  the Python-identity probe with a configurable ABI.
* `backend/crates/kala-cad-providers/tests/freecad_unit.rs` — three new tests
  (cp311–cp314 selection, single sys.path entry, unsupported reason).
* `backend/crates/kala-cad-providers/README.md` — ABI selection and test count.
* This report.

Generated artifacts (not source): `studio/backend-native/` (restaged; the
pre-change schema-1 stage was removed once because the new schema-2 validator
refuses the old layout during replacement), `studio/dist/`,
`studio/release/kala-studio-0.1.0-x86_64.AppImage`, `studio/release/linux-unpacked/`.

## 7. Notes and remaining gaps

1. **Windows is not executed here.** This host has no Windows and no `pwsh`.
   The `win_amd64` wheels for cp311–cp314 resolve and their digests are
   recorded, but the PowerShell staging/import path is a reviewed definition
   only. The user's Windows machine must run `build-windows.ps1` and the
   post-install checklist.
2. **No real FreeCAD build on Python 3.11–3.13 was run.** The official FreeCAD
   1.1.4 AppImage (py311) was not downloaded; cp311–cp313 adapter imports were
   proven with real matching CPython builds from `uv`, and cp314 with the host
   FreeCADCmd 3.14.8. A real FreeCAD 3.11 runtime remains the strongest missing
   check, though the path is the same code path proven for cp314.
3. The Python probe adds one extra `FreeCADCmd` invocation when ABI directories
   are present. It is cached with the existing availability cache (default
   30 s) and is not run for a development checkout.
4. Free-threaded tags (`cp313t`) are structurally recognised but not vendored
   (outside the cp311–cp314 set); they are reported unsupported by version.
5. The repository `adapters/freecad/vendor.json` matrix is regenerated
   explicitly (`--matrix`); it is not rewritten by every staging run, so a pin
   change needs that command plus a new staged build. The staging manifest is
   always regenerated and validated.
6. The AppImage is unsigned and not published; no signing material was read.
7. `libTK*`/OCCT staging and the Rust-only Electron launch were already proven
   by the packaging cell and were only re-exercised here; this cell did not
   change them.
