# The native Rust Kala Studio front door

This document describes the **Rust front door** of Kala Studio: what it
answers itself, how the geometry is now computed **natively** through the
real OpenCascade, what the supervised Python geometry worker has become
(a development-only fallback), and what the live differential proves. It
replaces the earlier description of the **nine-route** slice and the
**six-native/ten-proxied** split, neither of which is the shape of the
product any more.

> The per-module status of the whole Rust port — every Python module, its
> Rust home, its tests, its differential and review evidence, the declared
> differences and what is still open — is in
> [rust-migration-status.md](rust-migration-status.md). This document is the
> front door's own record.

The front door is the **Studio service** now (ADR-0010 phase 2). It computes
its own geometry, owns the session journal, and serves the packaged app with
**no Python process anywhere in the picture**. The Python Studio server
remains in the tree as the parity oracle; the Python geometry worker remains
as a development fallback. There is **no Windows runtime, no COM, no Solid
Edge, no code signing, no publication and no performance or readiness claim**
in this document beyond the evidence stated below.

Chat and design pass the operator's CAD provider through unchanged:
**FreeCAD builds** (with the live differential evidence and the extracted
AppImage proof), **Solid Edge refuses honestly on Linux**, and there is
**no fallback** provider.

## The two layers

1. **Rust front door** — `backend/crates/kala-studio/`. It owns the HTTP
   boundary (Host / Origin / bearer / body-size gates), the settings and
   token stores, the static UI, the deterministic local chat provider, the
   fixed-endpoint remote chat providers, the CAD-provider routing, **and the
   geometry**. Its default feature is `native-geometry`.
2. **React UI** — `studio/`, built by Vite into `studio/dist` and served
   same-origin by the front door.

### What the native geometry backend is

`src/native_geometry/` is the front door's geometry implementation. It is
the port of the Python `DesignService`, and it is backed by:

| Crate | What it owns |
|---|---|
| `kala-cad` | the **OpenCascade adapter**: compile, measure, tessellate, STEP write and read, hidden-line drawings and exact-area sections — one narrow C++ shim over the real OCCT 7.9.3 SDK |
| `kala-cad-providers` | the CAD provider **connections**: `FreeCADCmd` out of process, the Solid Edge loopback bridge |
| `kala-design-intent` | the `DesignIntent` contract, the part templates and the strict prompt parser |
| `kala-studio-journal` | the hash-chained session journal and the content-addressed artifact store (byte-compatible with the Python journal) |
| `kala-contracts` | the typed construction script and its canonical form |
| `kala-json`, `kala-persistence` | the canonical JSON and the lossless text rules |
| `kala-geometry`, `kala-step` | the reviewed analytic meshes and the analytic STEP writer (behind `native-primitives`, which `native-geometry` enables) — they answer the primitive `preview` / `export` / `handoff` routes |

**Geometry is never re-implemented in Rust.** `kala-cad` **calls** OpenCascade
through one owned C++ adapter (`src/ffi/adapter.cpp`); it does not rewrite
OCCT. `Shape` owns exactly one kernel handle and is deliberately
`!Send + !Sync`; every C++ entry point catches `Standard_Failure`,
`std::exception` and `...`, so no exception crosses the C ABI. The Studio's
own geometry lock serialises the design, drawing and section work, exactly
like the Python `_GEOMETRY_LOCK`.

The **Python reference server** (`python -m kala_agent.studio`) still exists
and still serves the same public Studio API without the Rust front door in
front. The differential harnesses compare the two.

## The route surface: 18 native, 0 proxied

The Python public server exposes **16** Studio route-methods. The front door
exposes **18**: the same 16 plus the ADR-0011 view-only `POST /import` and
the provider model catalog `GET /providers/{id}/models`, which the Python
public server does not mount. **All 18 are answered by Rust in-process** —
7 of them directly, and 11 through the **geometry-backend seam**, which the
default build also answers natively. The Python geometry worker is started
only when `KALA_STUDIO_GEOMETRY_WORKER` names one, and it is a development
fallback.

| Route | Owner |
|---|---|
| `GET /api/studio/health` | Rust |
| `GET /api/studio/integrations` | Rust |
| `GET /api/studio/providers` | Rust |
| `GET /api/studio/providers/{id}/models` | Rust (the provider model catalog) |
| `GET /api/studio/settings` | Rust |
| `POST /api/studio/settings` | Rust |
| `POST /api/studio/chat` | Rust |
| `POST /api/studio/preview` | Rust (the analytic primitives) |
| `POST /api/studio/export` | Rust (the analytic primitives) |
| `POST /api/studio/handoff` | Rust (the analytic primitives) |
| `POST /api/studio/design` | Rust (the native geometry backend) |
| `GET /api/studio/design/{id}/drawing` | Rust (the native geometry backend) |
| `GET /api/studio/design/{id}/section` | Rust (the native geometry backend) |
| `GET /api/studio/design/{id}/artifact/{kind}` | Rust (the native geometry backend) |
| `GET /api/studio/journal` | Rust (the native journal) |
| `GET /api/studio/journal/verify` | Rust (the native journal) |
| `GET /api/studio/providers/cad` | Rust (the provider connections) |
| `POST /api/studio/import` | Rust (the native view-only STEP import) |

`npm run check:native-readiness` parses the actual Python and Rust sources;
it does not trust a hardcoded list. It reports **READY** — **16** Python
route-methods covered, **7** answered directly and **11** through the
**geometry-backend seam**, which the default build answers natively (the
Python worker is opt-in). The built stage's report
(`studio/release/native-readiness.json`) classifies the artifact `release`
with `releaseEligible: true`. That file is a snapshot of the sources at
build time; the final 2026-10-21 build regenerated it with the current
split (11 route-methods through the geometry-backend seam).

The `GeometryBackend` trait is the seam. The production implementation is
`NativeGeometryBackend`; it answers with the same bytes the Python worker
answered with, and the front door forwards the request body and the original
content type unchanged. `worker_differential.py --backend worker` runs the
same **295-case** matrix against the supervised Python worker: **0
mismatches** as well.

## The geometry worker is now a development fallback

- **Command.** `KALA_STUDIO_GEOMETRY_WORKER` is either a JSON argv array
  (`["/path/python","-m","kala_agent.studio","--worker"]`) or a single
  executable path, in which case the fixed module arguments are appended.
  Whitespace splitting is never performed; a value that names an executable
  *and* arguments is refused rather than silently re-parsed. **The front door
  starts no worker at all unless this variable names one.**
- **Token.** 32 fresh random bytes rendered as 64 hexadecimal characters,
  passed **only** through `KALA_WORKER_TOKEN`. It is never in argv and never
  logged; `worker_lifecycle` reads `/proc/<pid>/cmdline` and asserts that
  neither the token nor the variable name appears.
- **Lifeline.** `KALA_WORKER_STDIN_LIFELINE=1` is set, so the worker stops
  when the front door closes its stdin. A dead front door cannot orphan a
  worker.
- **Readiness.** The worker prints exactly one `KALA_WORKER_READY {json}`
  line; the front door reads it with a bounded 30-second timeout. The port,
  version and `design_available` flag come from that line.
- **Restart.** A dead worker is restarted with capped exponential backoff
  (250 ms → 5 s). `/health` publishes
  `geometry_worker {state, version, design_available}`.
- **Degradation.** While the worker is down, the native routes keep working
  and the geometry-backend seam routes answer
  `503 geometry_worker_unavailable`. **No
  mesh, STEP file or measurement is fabricated** in that state.
- **Isolation.** The worker binds `127.0.0.1:0`, refuses any `Origin`
  header, requires the exact `Host`, and requires the per-launch bearer.

## Chat and design: the provider is passed through

`POST /chat` carries an optional `cad_provider`; `POST /design` carries a
required `provider`. The front door does **not** check availability itself
and does **not** substitute a provider. It passes the operator's choice
unchanged, and the native backend alone decides whether the provider can
build.

- With `cad_provider: "freecad"` the backend runs `FreeCADCmd` and builds the
  model. The live differential includes that real chat turn: design
  `a9d04f8ff14e…`, provider `freecad`, version `1.1.4`,
  `measured_by: "FreeCAD"`, volume **35571.416047 mm³**, area
  **11889.101731 mm²**, envelope 60 × 45 × 40 mm, one valid solid,
  `verified: true` (5/5 checks), 568 triangles / 586 vertices. The reply
  text is byte-identical to the Python server's.
- With `cad_provider: "opencascade"` the real kernel builds it (the
  `opencascade` opt-in provider is `kala-cad` itself).
- With `cad_provider: "solidedge"` on Linux the backend refuses with its own
  reason and the available providers. The front door relays that sentence
  verbatim, leaves `design: null`, and tries nothing else. No Solid Edge
  build has ever been executed on any host.
- With no `cad_provider`, the turn asks the operator to choose and builds
  nothing. OpenCascade is never selected automatically.

The cold-reload area fix lives in the native service too: a design restored
from the journal keeps its `surface_area_mm2`. See
[studio-limitations.md](studio-limitations.md#the-cold-reload-surface-area-bug-fixed).

## The shared STEP import ceiling

The view-only import has its own body ceiling, separate from the shared
256 KiB Studio limit. `KALA_STEP_IMPORT_MAX_BYTES` has the worker's own
semantics:

| Value | Result |
|---|---|
| absent or blank | the 64 MiB default |
| an integer in `1 … 536 870 912` (1 B … 512 MiB) | that value |
| anything else | a startup refusal, with the worker's own message |

The front door resolves the value **once, before the worker is launched**,
enforces it on the public import route, and hands the same number to the
worker process. Both sides therefore accept exactly the same body; the front
door never raises the worker's ceiling. The CPython `int()` grammar is
ported, including single underscores between digits. One documented
difference: non-ASCII decimal digits are refused by the Rust resolver with
the worker's message.

## The live differential

The release binary runs against the live Python public server (and, for
worker-only routes, a directly launched Python worker). Every harness was
re-run on this host on 2026-10-20 **with the native backend**:

| Harness | Result |
|---|---:|
| `crates/kala-studio/tests/worker_differential.py` | **295 cases, 0 mismatches, 0 known gaps** |
| `crates/kala-studio/tests/python_differential.py` | **268 cases, 0 mismatches** |
| `crates/kala-studio/tests/replay_binary_corpus.py` | **56 cases, 0 mismatches** |
| `crates/kala-studio/tests/python_provider_dto_oracle.py` | **20 cases, 0 failures** |

The corpus replay used to be **red** (5 mismatches) while the front door
proxied the geometry. The native backend brought the corpus' own declared
normalisation with it, and the harness now reports `exact: 48`, `fields: 4`
(the corpus' declared volatile paths), `step structural: 2`, `skipped: 2`
(the 16 384-byte library body-limit configuration, covered by
`corpus_fixture`) and **0 mismatches**. That replay ran against corpus
digest `c0003bae333f27a079adcf3e86f2618741a6606334a17ffe3fafe727fcd6c330`
(regenerated when the artifact-download route landed). The corpus was then
regenerated once more on 2026-10-21 (corpus-hermetic: a normalised body's
length is now recorded from the normalised bytes, so `--check` no longer
depends on the TMPDIR path length) to
`39e0c0a90f9d03704945c79cdb9e777e77549ef208ac2177d3f7bb003dade3fa`, the
value `backend/crates/kala-studio/tests/corpus_fixture.rs` pins now.
`studio_http.json`, which the replay reads, is byte-identical across that
regeneration, so the replay result stands. The analysis is in
[rust-migration-status.md §5.7](rust-migration-status.md).

The 295 cases include three ADR-0011 chat cases — FreeCAD, OpenCascade and
Solid Edge — compared on reply text, design, provider, version, every
measured value, the checks and the view. Status, body bytes and the stable
and security headers are compared exactly. Only these are normalised:

- the Rust-only `geometry_worker` health block (the Python server has no
  worker);
- the volatile journal fields (`occurred_at`, payload and chain digests, the
  journal head), because two processes never share a chain;
- the generated STEP/FCStd digests on the provider chat cases, because those
  writers embed their build time; the values and the reply text are still
  compared exactly;
- STEP downloads are compared structurally (a real ISO 10303-21 solid with
  stable headers), not byte for byte;
- the **`opencascade` provider version** and the **display mesh vertex
  order** — the two declared native differences (§ below). The FreeCAD and
  Solid Edge identities are still compared exactly.

### The two declared native differences

1. **The kernel binding.** The native build links the **system OCCT 7.9.3**;
   the Python oracle imports the `cadquery-ocp` binding 8.0.1.1. The
   reported `opencascade` provider version therefore names a different kernel
   binding, and the harness replaces that one field with a placeholder **for
   the `opencascade` provider only**.
2. **The display mesh vertex order.** The two OpenCascade versions assemble
   the same STEP file with its faces in a different order, so the imported
   triangulation is the same geometry in a different order. The harness
   compares the mesh by an **order-independent fingerprint** — the sorted
   `(position, normal)` vertices and the sorted triangles, plus the exact
   vertex count, the triangle count, the bounds, the solids and the warnings.
   The two fingerprints are equal: the same 586 vertices, the same 568
   triangles, the same bounds.

Run them from `backend/` with the repository venv:

```bash
../.venv/bin/python crates/kala-studio/tests/worker_differential.py
../.venv/bin/python crates/kala-studio/tests/python_differential.py
../.venv/bin/python crates/kala-studio/tests/replay_binary_corpus.py
../.venv/bin/python crates/kala-studio/tests/python_provider_dto_oracle.py
```

## Build and run

The release build is a root-workspace build:

```bash
cd backend
cargo build --release --locked -p kala-studio
```

Launch the front door with the built UI. **No worker, no Python, no virtual
environment:**

```bash
cd /home/v9i/Projects/Kala-Agent
export KALA_STUDIO_TOKEN=devtoken-0123456789abcdef
backend/target/release/kala-studio \
  --host 127.0.0.1 --port 8796 \
  --data-dir /tmp/kala-studio-data \
  --static-dist studio/dist
```

The token must be **16–128 characters**; a shorter one refuses startup. The
process prints one JSON launch banner, then serves on loopback. In another
shell:

```bash
curl -fsS -H "Authorization: Bearer devtoken-0123456789abcdef" \
  http://127.0.0.1:8796/api/studio/health
curl -fsS -H "Authorization: Bearer devtoken-0123456789abcdef" \
  http://127.0.0.1:8796/api/studio/providers/cad
```

The health document must contain `design_available: true` and
`step_available: true`; the provider probe must carry a live version or a
live reason for each of the three providers. A FreeCAD chat turn:

```bash
curl -fsS -H "Authorization: Bearer devtoken-0123456789abcdef" \
  -H "Content-Type: application/json" \
  -d '{"provider":"local","prompt":"Design a machined aluminum L-bracket for a 40 mm mounting plate, 8 mm wall thickness, with a 10 mm centered bore and 4x M5 corner mounting holes.","cad_provider":"freecad"}' \
  http://127.0.0.1:8796/api/studio/chat
```

This was run on this host: the turn returned `provider: "freecad"`,
`version: "1.1.4"`, volume `35571.416047 mm³`, area `11889.101731 mm²`,
`verified: true`, and stayed identical after a cold restart with the design
served `cached: true` and its area intact.

To develop against the **Python worker fallback** instead, set
`KALA_STUDIO_GEOMETRY_WORKER` explicitly:

```bash
export KALA_STUDIO_GEOMETRY_WORKER="[\"$PWD/.venv/bin/python\",\"-m\",\"kala_agent.studio\",\"--worker\"]"
```

## Native staging and packaging

`npm run build:native` runs the staging helper
(`studio/scripts/native-backend.cjs`), which builds the release `kala-studio`
binary and copies it with `studio/dist` into `studio/backend-native/`:

```
backend-native/
  kala-studio                 the Rust release runtime
  studio_dist/                the Vite UI
  native-build.json           digests, source parity, staging metadata
  lib/                        the exact ldd-resolved OCCT libTK*.so* (Linux)
  adapters/freecad/
    vendor.json               supported ABIs, wheel digests, detected FreeCAD
    src/kala_agent_freecad/   the adapter modules for FreeCADCmd
    src/kala_agent/           the shared pure-Python dependency
    site-packages-cp311/      the vendored wheels for FreeCAD's Python 3.11
    site-packages-cp312/      the vendored wheels for FreeCAD's Python 3.12
    site-packages-cp313/      the vendored wheels for FreeCAD's Python 3.13
    site-packages-cp314/      the vendored wheels for FreeCAD's Python 3.14
    mod/Kala/                 the FreeCAD workbench addon
```

The helper stages into a sibling temporary directory and swaps it in only
after the binary/UI digests, ownership markers and protected-destination
checks pass. It refuses Python/OCP/PyInstaller artifacts, interpreter
binaries, `libpython`, `site-packages` and any destination it does not own —
the only Python allowed inside the stage is the explicit
`adapters/freecad/**` bundle.

**OCCT shared libraries.** On Linux the staging runs `ldd` on the staged
binary and copies every resolved `libTK*.so*` (23 files here) plus any
non-system dependency outside `/lib*` and `/usr/lib*`, under the loader
name. With `patchelf` present (it is on the build host) the staged copy of
`kala-studio` gets `RUNPATH=$ORIGIN/lib` and each bundled library gets
`RUNPATH=$ORIGIN`; **the Cargo binary is never modified**. The extracted
runtime resolves all 23 libraries from `resources/backend/lib` with an
**empty environment and no `LD_LIBRARY_PATH`** — proved by `ldd` under
`env: {}`, not from the host's `/usr/lib`.

**The Electron launch is native-only.** In packaged mode the Electron main
process resolves `resources/backend/kala-studio[.exe]` **only**. It never
consults `KALA_STUDIO_PYTHON`, `KALA_STUDIO_DEV_BACKEND`, a PATH Python or
a repository virtualenv, and it removes `PYTHONPATH`, `PYTHONHOME`,
`KALA_STUDIO_PYTHON`, `KALA_STUDIO_DEV_BACKEND` and
`KALA_STUDIO_GEOMETRY_WORKER` from the child environment. In development
the staged native slice is the default too; the Python reference is reachable
only behind an explicit `KALA_STUDIO_DEV_BACKEND=python` (or a
`KALA_STUDIO_PYTHON` override).

**The release gate** is the native-design readiness report
(`desktop-pack.cjs` → `native-readiness.cjs`), replacing the old
`nativeStep` feature check. `desktop-pack` additionally refuses a stage that
lacks the bundled FreeCAD adapter, and publication is always forced to
`never`.

### The Linux AppImage (built and smoke-tested)

| Field | Value |
|---|---|
| Path | `studio/release/kala-studio-0.1.0-x86_64.AppImage` |
| Size and SHA-256 | see [packaging/linux/README.md](../packaging/linux/README.md#the-built-artefact-size-and-sha-256) — the digest changes with every rebuild, so it is recorded there only |
| Classification | `release`, `releaseEligible: true` in `studio/release/native-readiness.json` |
| Signature | **none** — the AppImage is **unsigned** and **not published** |

Build chain (all exit 0):

```bash
cd studio
npm run build
node scripts/native-backend.cjs --require-freecad-vendor
node scripts/desktop-pack.cjs --linux AppImage --x64 --publish never
```

`packaging/linux/build-appimage.sh` wraps exactly that and prints the
artifact path, size and sha256.

### The smoke (re-run under a literal `env -i`)

`xvfb-run` is **not** installed on this host, so the smoke uses
`--appimage-extract` (no FUSE, no X server) and runs
`resources/backend/kala-studio` directly in two phases, each with an
explicit child environment and nothing inherited:

```sh
env -i PATH=/usr/bin:/bin HOME=<tmp> TMPDIR=<tmp> XDG_DATA_HOME=<tmp> \
  KALA_AGENT_DATA_DIR=<tmp> KALA_STUDIO_DATA_DIR=<tmp> \
  /path/to/node packaging/linux/smoke-appimage.mjs --require-geometry --require-freecad
```

**Phase 1 — `PATH=""` (no Python on `PATH` at all):**

| Check | Result |
|---|---|
| payload | metadata schema 3, 23 OCCT libs, the FreeCAD adapter + 4 ABI sets, **no Python runtime** |
| RPATH closure | 23 `libTK*` resolved from `resources/backend/lib` with an empty environment |
| `/api/studio/health` | `status: ok`, `design_available: true`, `step_available: true`, `geometry_worker {state: ready, version: 7.9.3}` |
| packaged UI | `GET /` serves HTML |
| auth | unauthenticated `POST /api/studio/preview` → 401 |
| OpenCascade L-bracket | **35571.416047113315 mm³**, bounds `[-30,0,0] … [30,45,40]`, `measured_by: OpenCascade (opt-in)` |
| drawing | `front(7) top(11) right(8)` visible segments |
| section | XZ at `y=4`: **2321.4601836602546 mm² = 2400 − 25π**, `area_exact: true`, 1 outer + 1 inner loop |
| STEP round trip | export 55 554 bytes; import returns 1 solid, same digest, bounds equal within 0.001 mm |

**Phase 2 — `PATH=/usr/bin:/bin:/usr/local/bin`, a temp cwd, no
`PYTHONPATH`, no `PYTHONHOME`, no `KALA_FREECAD_SYS_PATH`, no repository and
no virtualenv in any env value:**

| Check | Result |
|---|---|
| FreeCAD phase health | ok |
| provider list | `freecad` available, version `1.1.4` |
| FreeCAD L-bracket | **35571.416047 mm³**, bounds `[-30,-8.88e-16,0] … [30,45,40]`, `measured_by: FreeCAD` |
| drawing | `front(7) top(11) right(8)` |
| section | the same exact area 2321.4601836602546 mm², 1/1 loops |
| STEP round trip | export 21 648 bytes; import 1 solid, bounds equal within 0.001 mm |

**FreeCAD is detected on the system and is not bundled.** The runtime
probes `FreeCADCmd`'s own Python (a cheap, cached probe) and puts exactly
the matching `site-packages-cpNNN` directory on `sys.path`, so one AppImage
works with FreeCAD builds on Python 3.11, 3.12, 3.13 and 3.14. An
unsupported Python is reported as provider-**unavailable** with its exact
version — never as a crash.

### The Windows build (written, **not executed**)

`packaging/windows/build-windows.ps1` produces the NSIS installer from
MSVC + vcpkg `opencascade:x64-windows` + Node 22. It stages the `TK*.dll`
closure next to `kala-studio.exe`, vendors the four `win_amd64` ABI sets,
and prints the artifact hash and the post-install checklist.

**It has never been executed.** This host has no Windows and no `pwsh`; the
script is a **definition, not a Windows-runtime proof**. The user's Windows
machine runs the build and the checklist. The full instructions are in
[studio-install.md](studio-install.md) and
`packaging/windows/README.md`.

## Verification on this host

Linux, Python 3.14.8, OCP 8.0.1.1, OCCT 7.9.3, FreeCAD 1.1.4,
rustc/cargo 1.99.1, Node 26.8.2, npm 11.19.1.

### Run in this docs pass (2026-10-20)

| Command | Result |
|---|---|
| `cd backend && cargo test --workspace --locked --release` | **449 passed**, 0 failed, 0 ignored (of which `kala-studio` is 175) |
| `cd backend/crates/kala-cad && cargo test --locked --release` | **118 passed** (75 after the core, +43 from the HLR drawing and the section) |
| `cd backend/crates/kala-studio-journal && cargo test --locked --release` | **60 passed** |
| `cd backend/crates/kala-cad-providers && cargo test --locked --release` | **40 passed** |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/worker_differential.py` | **295 cases, 0 mismatches, 0 known gaps** |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/python_differential.py` | **268 cases, 0 mismatches** |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/python_provider_dto_oracle.py` | **20 cases, 0 failures** |
| `cd backend && ../.venv/bin/python crates/kala-studio/tests/replay_binary_corpus.py` | **56 cases, 0 mismatches**; corpus digest `c0003bae…` |
| `cd studio && npm run typecheck` | **exit 0** |
| `cd studio && npm test` | **43 files, 628 tests passed** |
| `cd studio && npm run build` | **exit 0** (the pre-existing `three` >500 kB warning) |
| `env -i … node packaging/linux/smoke-appimage.mjs --require-geometry --require-freecad` | **PASS** (both tables above) |
| `sha256sum studio/release/kala-studio-0.1.0-x86_64.AppImage` | see [packaging/linux/README.md](../packaging/linux/README.md#the-built-artefact-size-and-sha-256) — the digest moves with every rebuild |

### Carried over from the previous docs pass (not re-run here)

| Command | Result then |
|---|---|
| `cd backend && cargo fmt --all --check` | clean |
| `cd backend && cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| `cd backend && cargo build --workspace --release --locked` | success |
| `cd studio && npm run check:native-readiness` | READY; the built stage's `studio/release/native-readiness.json` classifies the artifact `release` / `releaseEligible: true` |
| the launch, health, providers, FreeCAD chat, Solid Edge refusal, no-provider and cold-reload curls | all as described above; journal verify `ok: true` |

**Not run in this docs pass** (and not claimed): `cargo fmt`/`clippy` per
crate, `npm ci`, `pip install -e ".[studio]"` / `".[geometry]"`,
`npm run desktop`, the **AppImage build** (the existing artifact was
verified, not rebuilt), `packaging/windows/build-windows.ps1` (no Windows, no
`pwsh`), the Windows cross-check (`cargo check -p kala-studio --target
x86_64-pc-windows-gnu` is blocked by the missing MinGW C toolchain that
`ring` needs), any Solid Edge build, and the Electron **window** rendering
(no `xvfb-run` on this host).

## Honest limits

- The front door **is** the Studio service now, and it computes its own
  geometry, journal and provider connections. The Python Studio server and
  the Python worker remain in the tree as the parity oracle and the
  development fallback; nothing was deleted, and the Python reference is
  still the oracle the differentials compare against.
- **Solid Edge is unproven.** On Linux it refuses honestly; on Windows the
  ADR-0008 bridge and the COM runtime have never been exercised here, and
  the bridge still has no verified native build executor.
- **Windows is unexecuted.** The NSIS script is written and reviewed; the
  build, the install and the checklist are the user's machine's job. No
  Windows runtime claim is made.
- **The AppImage is unsigned and unpublished**, and the release workflow was
  written but not pushed or run.
- **The Electron window was never rendered** on this host (`xvfb-run` is
  absent); the packaged backend and the served UI root are verified by
  extraction instead.
- **No performance, memory or installer claim** is made. No benchmark was
  run.
- Generated STEP/FCStd digests are build-time artifacts; never use a
  cross-process digest as a parity oracle. Compare size, geometry and the
  front door's own receipt instead.
- The agent runtime is **not** part of the front door. It is now ported and
  accepted in its own right (`kala-runtime`, 112 tests), but it is not
  composed into the front door and no runtime cutover is claimed.
- The frozen corpus's **host-dependent** records are not a contract. The
  `studio.cad_providers` case records the CAD-provider probe of the machine
  that generated the corpus; a different host answers differently, and that
  is a fact of the corpus, not a defect of the front door.