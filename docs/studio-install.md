# Installing and launching Kala Studio

> Scope: end-user installation, developer workflow, and CI artefacts.
> This document describes how the wave builds the desktop bundle. It
> does **not** edit the user's system configuration; running the
> launcher creates files only inside the application data directory
> and the desktop bundle's own resources.
>
> **Current state (2026-10-20).** The desktop app is **Rust-only**
> (ADR-0010 phase 2): the packaged application starts the native
> `kala-studio` binary and **no Python process at all**. The **Linux
> AppImage exists and has been smoke-tested**; the **Windows installer
> is a script that has not been executed yet** (this host has no
> Windows), so no Windows runtime is claimed.

## The deliverables at a glance

| Deliverable | State |
|---|---|
| **Linux AppImage** | **Built and smoke-tested.** `studio/release/kala-studio-0.1.0-x86_64.AppImage`, **163 642 516 bytes** (156.1 MiB), sha256 `b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e`. **Unsigned** and not published. |
| **Windows NSIS installer** | **Not built.** `packaging/windows/build-windows.ps1` is written and reviewed, but it has **never been executed** — there is no Windows and no `pwsh` on the build host. The user's Windows machine runs it. |
| **FreeCAD** | **Not bundled.** It is detected on the system (`FreeCADCmd` on `PATH`). The app ships the adapter and its Python dependencies for **cp311, cp312, cp313 and cp314**, so the installed app needs **no virtualenv** and no `PYTHONPATH`. |
| **Solid Edge** | Windows only, and still **unproven** — no Solid Edge build has ever been executed on any host. |

## Ways to run the studio

### 1. The Linux AppImage (the built deliverable)

The AppImage is the consumer-facing Linux entry point. It is a single,
self-contained file; no system-level installation is performed, and it runs
in its own filesystem mount.

```bash
chmod +x kala-studio-0.1.0-x86_64.AppImage
./kala-studio-0.1.0-x86_64.AppImage
```

If the host uses FUSE 3 and the AppImage fails to mount, use
`--appimage-extract-and-run`.

**What is inside it** (and what is not):

| Inside | Detail |
|---|---|
| `resources/backend/kala-studio` | the native Rust runtime, linking **OpenCascade 7.9.3** |
| `resources/backend/lib/` | the exact `ldd`-resolved OCCT closure — **23 `libTK*.so*`**, staged under the loader name with `RUNPATH=$ORIGIN/lib` (`patchelf` 0.19.2). The Cargo binary itself is never modified. |
| `resources/backend/studio_dist/` | the packaged UI |
| `resources/backend/adapters/freecad/` | the FreeCAD adapter, the shared `kala_agent` tree and **four per-ABI wheel sets** (`site-packages-cp311` … `site-packages-cp314`) |
| `resources/backend/adapters/freecad/mod/Kala` | the FreeCAD workbench addon (staged, **not** auto-installed into FreeCAD) |
| `resources/backend/native-build.json` | the stage metadata: digests, the library closure, the wheel digests, the source parity |
| **Not inside** | any Python interpreter, `libpython`, `site-packages` or PyInstaller payload. The only Python in the package is the FreeCAD adapter, which runs **inside the user's own FreeCAD**. |

The first launch picks a free loopback port, generates a random
`KALA_STUDIO_TOKEN`, writes it to a launcher file with restricted
permissions (0600 on POSIX), starts the bundled backend, waits up to
15 s for `/api/studio/health` to return `{status:"ok"}`, and opens a
desktop window pointing at the loopback URL. The bearer token is
injected into the window via a single Electron IPC channel.

**The app writes its data directory** under `$XDG_DATA_HOME/kala-studio/`
(default `~/.local/share/kala-studio/`). The bearer token file is created
with mode 0600.

> **Not exercised on the build host:** `xvfb-run` is not installed, so the
> Electron **window rendering** was never validated. The smoke test verifies
> the packaged **backend** and the **served UI root** by extracting the image
> (`--appimage-extract`, no FUSE, no X server) and running the binary
> directly. A generated AppImage is not proof that a desktop session works
> on a clean Linux host.

### 2. Developer mode — the native Rust front door

This is the path ADR-0010 phase 2 describes: a native Rust front door that
owns the HTTP boundary **and the geometry**. There is **no Python worker**,
no virtualenv and no `pip install` in the default path.

```bash
# from a clean checkout
cd backend
cargo build --release -p kala-studio --locked
cd ..

# build the UI once
cd studio
npm ci
npm run build
cd ..

# run it — no worker, no Python
export KALA_STUDIO_TOKEN=devtoken-0123456789abcdef     # 16–128 characters
backend/target/release/kala-studio \
  --host 127.0.0.1 --port 8765 \
  --data-dir /tmp/kala-studio-data \
  --static-dist studio/dist
```

Check it:

```bash
curl -fsS http://127.0.0.1:8765/api/studio/health
curl -fsS -H "Authorization: Bearer devtoken-0123456789abcdef" \
  http://127.0.0.1:8765/api/studio/providers/cad
```

The health document must carry `design_available: true` and
`step_available: true`. The provider route must answer the three CAD
providers with a live version or a live reason.

`KALA_STEP_IMPORT_MAX_BYTES` sets the one STEP import ceiling for the
front door (default 64 MiB, allowed range 1 B … 512 MiB). An unusable value
refuses startup with exit code 2.

**The Python geometry worker is a development-only opt-in.** It is started
**only** when `KALA_STUDIO_GEOMETRY_WORKER` names one:

```bash
export KALA_STUDIO_GEOMETRY_WORKER="[\"$PWD/.venv/bin/python\",\"-m\",\"kala_agent.studio\",\"--worker\"]"
```

The value is **either** a JSON argv array (as above) **or** a single
executable path, in which case the fixed arguments `-m kala_agent.studio
--worker` are appended. Whitespace splitting is **not** performed: a value
that names an executable *and* arguments is refused with a clear message.
There is **no development default** any more — an unset variable means no
worker at all.

### 3. Developer mode — the Python reference server

The Python server still serves the same public API without the Rust
front door in front, and is the parity oracle the differential harnesses
compare against:

```bash
# in one terminal
export KALA_STUDIO_TOKEN=devtoken-0123456789abcdef
PYTHONPATH=src .venv/bin/python -m kala_agent.studio \
  --host 127.0.0.1 --port 8765 \
  --data-dir /tmp/kala-studio-data \
  --static-dist studio/dist

# in a second terminal
cd studio
npm run build
# open http://127.0.0.1:8765/ in a browser
```

Without a token argument, the server prints a randomly generated
`KALA_STUDIO_TOKEN` to the terminal. Copy it once per session and
paste it into the browser fallback prompt. The server never returns
the token over HTTP and never logs it.

### 4. Browser fallback (no desktop bundle)

This is not a separate back end: it is the same front door (or the same
reference server) opened in a browser instead of in Electron.

For headless or browser-only use:

1. Run the front door (or the reference server) in a terminal.
2. Note the printed token.
3. Open `http://127.0.0.1:8765/` in a same-origin browser tab.
4. Paste the token into the UI's "Manual token" field. The token is
   held in `sessionStorage` only; closing the tab discards it.

Same-origin browser fallback does not bypass the loopback check; the
server still rejects cross-origin requests.

## Building the Linux AppImage yourself

```bash
export RUSTUP_HOME=/home/v9i/.rustup
export CARGO_HOME=/home/v9i/.cargo
(cd studio && npm ci)
packaging/linux/build-appimage.sh
```

The script runs `npm run build`, the native staging
(`node scripts/native-backend.cjs --require-freecad-vendor`), then
`node scripts/desktop-pack.cjs --linux AppImage --x64 --publish never`,
prints the artifact path/size/sha256 and runs the headless smoke test. Use
`--no-smoke` to stop after the artifact.

The equivalent manual chain is:

```bash
cd studio
npm run build
node scripts/native-backend.cjs --require-freecad-vendor
node scripts/desktop-pack.cjs --linux AppImage --x64 --publish never
```

The release gate is the native-design readiness report; a stage that lacks
the bundled FreeCAD adapter is refused and publication is forced to
`never`.

### The headless smoke

```bash
env -i PATH=/usr/bin:/bin HOME=$(mktemp -d) TMPDIR=/tmp \
  XDG_DATA_HOME=$(mktemp -d) KALA_AGENT_DATA_DIR=$(mktemp -d) \
  KALA_STUDIO_DATA_DIR=$(mktemp -d) \
  /path/to/node packaging/linux/smoke-appimage.mjs \
    --appimage studio/release/kala-studio-0.1.0-x86_64.AppImage \
    --require-geometry --require-freecad
```

Phase 1 runs the backend with `PATH=""` (no Python at all) and checks the
health document, the served UI, the 401 for an unauthenticated preview, the
OpenCascade L-bracket (`volume_mm3 = 35571.416047`, rel 1e-6, bounds
`[-30,30] × [0,45] × [0,40]`), the HLR drawing (front/top/right), the XZ
section at `y = 4` (area `2400 − 25π`, `area_exact = true`, 1 outer + 1
inner loop) and the STEP export/import round trip (bounds within 0.001 mm).
Phase 2 puts FreeCAD on the `PATH` and runs the FreeCAD provider from the
extracted app with no repository, no virtualenv and no `PYTHONPATH`.

## Building and installing on Windows

> **The script has not been run.** Everything below is the *procedure*; the
> build, the install and the post-install checklist happen on the user's
> Windows machine. This host has no Windows and no `pwsh`, so nothing here
> is a Windows-runtime proof.

### Prerequisites

| Requirement | Version / notes |
|---|---|
| Windows | 10/11 x64 |
| Visual Studio Build Tools | 2022, "Desktop development with C++" workload (MSVC + Windows SDK) |
| Rust | rustup with the `x86_64-pc-windows-msvc` toolchain (`rustup default stable-msvc`) |
| Node.js | 22.12 or newer (the Studio `engines` requirement) |
| Git | any current version |
| vcpkg | with `opencascade:x64-windows` installed |
| FreeCAD (optional at build time) | 1.1.x x64; if installed, the script verifies that the bundled adapter imports inside that exact `FreeCADCmd` Python. Needed at runtime for the FreeCAD provider. |

One-time vcpkg setup (PowerShell):

```powershell
git clone https://github.com/microsoft/vcpkg C:\vcpkg
C:\vcpkg\bootstrap-vcpkg.bat
C:\vcpkg\vcpkg.exe install opencascade:x64-windows
$env:VCPKG_ROOT = "C:\vcpkg"   # or set it permanently
```

### Run the build script

From the repository root:

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-windows.ps1
```

With the full Studio gates first:

```powershell
powershell -ExecutionPolicy Bypass -File packaging\windows\build-windows.ps1 -RunGates
```

Useful switches:

* `-VcpkgRoot C:\vcpkg` — when `VCPKG_ROOT` is not set.
* `-VcpkgTriplet x64-windows` — must match the triplet you installed.
* `-FreeCadExe "C:\Program Files\FreeCAD 1.1\bin\FreeCADCmd.exe"` — explicit
  FreeCAD interpreter for the in-FreeCAD import check (otherwise
  `FreeCADCmd.exe` on `PATH`, then `%ProgramFiles%\FreeCAD*\bin` is used).
  All four supported ABIs are vendored whether or not FreeCAD is installed.
* `-FreeCadPythonVersion 3.14 -FreeCadAbi cp314` — development aid only, and
  only when FreeCAD is not installed: restrict the vendored set to one
  interpreter instead of cp311-cp314. The resulting installer is single-ABI.
* `-SkipNpmInstall` — reuse an existing `studio\node_modules`.
* `-RunGates` — run `npm run typecheck` and `npm test` before building.
* `-SkipBuildTests` — with `-RunGates`, run typecheck only.

### What the script does

It stops on the first failure:

1. Verifies rustc/cargo are the MSVC host, Node is 22.12+, and the vcpkg OCCT
   headers (`Standard_Version.hxx`) and `TKernel.dll` exist.
2. `npm ci` in `studio\` (unless `-SkipNpmInstall`).
3. Optional typecheck/tests.
4. `npm run build` — the Vite production UI.
5. Detects FreeCAD's bundled Python: finds `FreeCADCmd.exe`, runs a probe
   script in that interpreter and reads `sys.version_info`, the cache tag and
   `sysconfig.get_platform()` (for example FreeCAD 1.1.4 → Python 3.14.8,
   `cpython-314`, `win-amd64`).
6. `node scripts\native-backend.cjs --occt-lib-dir <vcpkg bin>
   --freecad-python <FreeCADCmd> --require-freecad-vendor` — builds the
   locked release `kala-studio.exe` and stages the binary, the UI, the vcpkg
   `TK*.dll` closure **next to the executable**, the FreeCAD adapter, the
   `mod\Kala` addon, the four `win_amd64` ABI wheel sets and
   `native-build.json`.
7. Verifies `vendor.json` is schema 2 with all four ABIs, that each ABI
   directory has the vendored packages and a `_pydantic_core*.pyd`, that the
   detected interpreter's ABI is in the set, and that at least one `TK*.dll`
   was staged.
8. `node scripts\desktop-pack.cjs --win nsis --x64 --publish never` —
   electron-builder copies the stage to `resources\backend\` and builds the
   NSIS installer.
9. Prints the installer path, size and SHA-256, and the post-install smoke
   checklist.

Output: `studio\release\kala-studio-<version>-setup.exe`.

### The installed layout

```
Kala Studio\
  Kala Studio.exe              Electron shell
  resources\
    app.asar                   electron/bootstrap|main|preload
    backend\
      kala-studio.exe          native Rust runtime
      studio_dist\             packaged UI
      native-build.json
      TK*.dll + dependencies   OCCT runtime next to the executable
      adapters\freecad\
        vendor.json              ABIs, detected FreeCAD, wheel digests
        src\kala_agent_freecad\  adapter modules used by FreeCADCmd
        src\kala_agent\          shared pure-Python dependency
        site-packages-cp311\     vendored wheels for FreeCAD's Python 3.11
        site-packages-cp312\     vendored wheels for FreeCAD's Python 3.12
        site-packages-cp313\     vendored wheels for FreeCAD's Python 3.13
        site-packages-cp314\     vendored wheels for FreeCAD's Python 3.14
        mod\Kala\                FreeCAD workbench addon
```

### Installing and running

The build produces an **unsigned** NSIS installer named
`kala-studio-<version>-setup.exe` (user-facing). The installer embeds an
unsigned `.nsis.7z` payload, which is an internal archive and **not** an
installer to run directly. The desktop shell launches
`resources/backend/kala-studio.exe`. The user must accept the publisher
warning ("Unknown publisher") because the build is not code-signed.

Steps on a clean Windows 10 / 11 host:

1. Run the unsigned `kala-studio-<version>-setup.exe`.
2. Right-click it -> Properties -> tick "Unblock" if SmartScreen flagged
   the download.
3. Run the installer and accept the publisher warning ("Unknown
   publisher"). The build is unsigned; the warning is expected.
4. Right-click and "Run as administrator" is **not** required and is
   **not** recommended. The app runs in user mode.
5. The window opens to the studio workspace.
6. Close the window to shut the backend down.

No Windows registry keys are written. No Windows services are installed. No
scheduled tasks are created. The wrapper does not modify `PATH` or any shell
startup file.

### The post-install smoke checklist

Run this on the Windows machine after installing:

1. Install as the current user (not elevated).
2. Set a throwaway data directory for the first launch:
   `set KALA_AGENT_DATA_DIR=%TEMP%\kala-studio-smoke-data` and
   `set KALA_STUDIO_DATA_DIR=%TEMP%\kala-studio-smoke-data`.
3. Launch Kala Studio. Task Manager must show exactly one `kala-studio.exe`
   child and **no `python.exe`**.
4. The window must load the workspace from `http://127.0.0.1:<port>/`.
5. Health: `curl.exe http://127.0.0.1:<port>/api/studio/health` → `status: ok`
   with `design_available: true`.
6. OpenCascade L-bracket: `volume_mm3 = 35571.416047` (rel 1e-6), bounds
   `[-30,30] × [0,45] × [0,40]`.
7. FreeCAD L-bracket (FreeCAD 1.1.x installed; **not** bundled): the provider
   must be available and the design must build in a real `FreeCADCmd`
   subprocess (`measured_by: FreeCAD`) with the same volume. The installed
   app supplies the adapter from `resources\backend\adapters\freecad\src`
   and the matching `site-packages-cpNNN` directory; do **not** add the
   repository virtualenv or a `PYTHONPATH` — no venv is needed. A FreeCAD
   build with Python 3.11–3.14 is supported; an unsupported Python appears
   as an unavailable reason naming its exact version.
8. STEP export and re-import: the view bounds match within 0.001 mm.
9. Close the window: `kala-studio.exe` exits; no orphan process remains.
10. Uninstall: the user data directory is left in place.

## Dev commands (studio/)

The Electron + React workspace owns its own package.json. Verify the
exact scripts in `studio/package.json`; the names below are the
ones this wave assumes.

```bash
cd studio
npm ci                # install deps, reproducible lockfile
npm run typecheck     # TypeScript strict mode over studio/src
npm test              # 43 test files, 628 tests (at the time of writing)
npm run build         # Vite production build into studio/dist
npm run desktop       # launch electron + backend in dev mode
npm run desktop:pack  # electron-builder; reads studio/backend-native/
```

The dev launcher stages the **native** slice by default. The Python
reference is reachable only behind an explicit
`KALA_STUDIO_DEV_BACKEND=python` (or a `KALA_STUDIO_PYTHON` override).

Two more scripts matter for the Rust front door:

```bash
cd studio
npm run check:native-readiness   # route-surface parity report
npm run build:native             # stage the native runtime + dist + OCCT libs
```

It parses the real Python and Rust route tables and reports the parity
split. The built stage's report (`studio/release/native-readiness.json`)
classifies the artifact `release` with `releaseEligible: true`.

## Dev commands (backend/)

From `backend/`:

```bash
cargo build --release -p kala-studio --locked        # the front door + native geometry
cargo test --workspace --locked --release            # the root workspace (449 tests)
cargo clippy -p kala-studio --all-targets --all-features --locked -- -D warnings
```

The live differentials need the repository venv (they drive the OCP
kernel):

```bash
cd backend
../.venv/bin/python crates/kala-studio/tests/worker_differential.py
../.venv/bin/python crates/kala-studio/tests/python_differential.py
../.venv/bin/python crates/kala-studio/tests/replay_binary_corpus.py
../.venv/bin/python crates/kala-studio/tests/python_provider_dto_oracle.py
```

`cargo check -p kala-studio --target x86_64-pc-windows-gnu` is
**blocked on this host**: `ring`'s build script needs
`x86_64-w64-mingw32-gcc`, which is not installed. That is a missing
C toolchain, not a defect in the crate, and no Windows compile or
runtime claim is made. The OCCT-linked Windows build additionally needs a
Windows OCCT SDK (`OCCT_INCLUDE_DIR` / `OCCT_ROOT` / `VCPKG_ROOT`), which
the vcpkg installation in the script above provides.

## What the desktop installer does not do

- It does not register a Windows service.
- It does not install a Linux package (.deb, .rpm, Arch PKGBUILD).
- It does not write to `/etc/`, `/usr/`, or any system-wide path.
- It does not modify the host's `PATH`, `LD_LIBRARY_PATH`, or
  Electron update channel.
- It does not enable Auto-update; new versions must be downloaded
  manually.
- It does not request any capability beyond the loopback TCP port
  it printed at startup.
- It does not bundle FreeCAD, and it does not install the `mod\Kala`
  workbench addon into FreeCAD for you.

If a future build needs system-level integration, it must be
documented in this file with explicit user consent.

## Troubleshooting checklist

| Symptom | First thing to check |
| ---|---|
|  Window opens then immediately closes | Backend did not respond to `/api/studio/health` within 15 s. Check the launcher log file in `~/.local/share/kala-studio/log/` or `%APPDATA%\kala-studio\log\`. |
|  401 on every request | Token mismatch between Electron IPC and the backend. Restart the desktop; the token is regenerated each launch. |
|  503 on STEP export | The native OCCT runtime could not execute. The health document's `design_available` and `geometry_worker` blocks say why; a portable build (`--no-default-features`) reports `design_available: false`. |
| 503 `geometry_worker_unavailable` | Only reachable in the **development fallback**: a worker was named by `KALA_STUDIO_GEOMETRY_WORKER` but did not start. Check that the value is a JSON argv array (or a single executable path) and that the venv has `cadquery-ocp`. The native routes keep working; no measurement is served while the worker is down. |
| The worker exits immediately | `KALA_WORKER_TOKEN` must be at least 32 characters from `[A-Za-z0-9._~-]`. The front door mints it; the worker refuses to start without it and prints nothing to stdout. |
| A design turn asks which provider to use | No CAD target was selected. Use the CAD-target selector in the composer; there is deliberately no default provider. |
| A provider row is disabled | The live probe says it is unavailable and the row carries the backend's reason. FreeCAD needs `FreeCADCmd` on `PATH`; Solid Edge needs a Windows host with the bridge. |
| FreeCAD says it is unavailable and names a Python version | The bundled adapter dependencies cover **cp311–cp314**. An unsupported interpreter (for example 3.10) is reported honestly rather than crashing; install a FreeCAD build with a supported Python. |
|  Browser fallback prompts for token but the token was lost | Restart the backend and copy the freshly printed token. The backend never stores the token in any persistent log. |
|  AppImage fails to mount on a host without FUSE | Use `--appimage-extract-and-run` once; the bundle still works. |
|  CI artefact is missing on the GitHub Actions run | The matrix is gated on `ubuntu-latest` and `windows-latest`. Inspect the failed job's log; the wave does not retry or fallback to a runner that is not on the matrix. |
|  The Windows installer was never produced | `packaging/windows/build-windows.ps1` has **not** been executed anywhere yet. Run it on the Windows machine with the prerequisites above; the post-install checklist is the proof. |
