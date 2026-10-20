# Windows packaging (MSVC + NSIS)

`build-windows.ps1` produces the Windows installer for Kala Studio. The
installed application launches only the native Rust `kala-studio.exe`; no
Python interpreter is bundled or started. The only Python source in the package
is the FreeCAD adapter under `resources\backend\adapters\freecad\`, which runs
inside the user's own FreeCAD installation.

## Prerequisites

| Requirement | Version / notes |
|---|---|
| Windows | 10/11 x64 |
| Visual Studio Build Tools | 2022, "Desktop development with C++" workload (MSVC + Windows SDK) |
| Rust | rustup with the `x86_64-pc-windows-msvc` toolchain (`rustup default stable-msvc`) |
| Node.js | 22.12 or newer (the Studio `engines` requirement) |
| Git | any current version |
| vcpkg | with `opencascade:x64-windows` installed |
| FreeCAD (optional at build time) | 1.1.x x64; if installed, the script verifies the bundled adapter imports inside that exact `FreeCADCmd` Python. Needed at runtime for the FreeCAD provider |

One-time vcpkg setup (PowerShell):

```powershell
git clone https://github.com/microsoft/vcpkg C:\vcpkg
C:\vcpkg\bootstrap-vcpkg.bat
C:\vcpkg\vcpkg.exe install opencascade:x64-windows
$env:VCPKG_ROOT = "C:\vcpkg"   # or set it permanently
```

The Rust build discovers OCCT from `KALA_OCCT_ROOT`,
`KALA_OCCT_INCLUDE_DIR` and `KALA_OCCT_LIB_DIR` (see
`backend/crates/kala-cad/build.rs`). The script sets all three to
`%VCPKG_ROOT%\installed\x64-windows` for the build; you do not need to set
them by hand.

## Build

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

The script performs these steps and stops on the first failure:

1. Verifies rustc/cargo are the MSVC host, Node is 22.12+, and the vcpkg OCCT
   headers (`Standard_Version.hxx`) and `TKernel.dll` exist.
2. `npm ci` in `studio\` (unless `-SkipNpmInstall`).
3. Optional typecheck/tests.
4. `npm run build` — Vite production UI.
5. Detects FreeCAD's bundled Python: finds `FreeCADCmd.exe`, runs a probe
   script in that interpreter, and reads `sys.version_info`, the cache tag and
   `sysconfig.get_platform()` (for example FreeCAD 1.1.4 → Python 3.14.8,
   `cpython-314`, `win-amd64`). Detection selects the interpreter used for the
   import check; it does not restrict the vendored set.
6. `node scripts\native-backend.cjs --occt-lib-dir <vcpkg bin>
   --freecad-python <FreeCADCmd> --require-freecad-vendor` — builds the locked
   release `kala-studio.exe` and stages:
   * `studio\backend-native\kala-studio.exe`
   * `studio\backend-native\studio_dist\`
   * `studio\backend-native\TK*.dll` — every vcpkg DLL **next to
     `kala-studio.exe`** (Windows loader search order finds them there; the
     Electron main process also prepends the backend directory to the child
     `PATH`).
   * `studio\backend-native\adapters\freecad\src\kala_agent_freecad\` and
     `mod\Kala\`
   * `adapters\freecad\vendor.json` plus the vendored Python dependencies:
     `kala_agent` once under `src\`, and one
     `adapters\freecad\site-packages-cpNNN\` for each of cp311, cp312, cp313
     and cp314 (`pydantic`, `pydantic_core`, `annotated_types`,
     `typing_inspection`, `typing_extensions`). Each set is downloaded as
     `win_amd64` wheels with `pip download --only-binary=:all:
     --python-version 3.X --abi cp3XX`; the build host's ABI set is also
     imported inside `FreeCADCmd`.
   * `native-build.json` with binary/static digests, source parity, library and
     wheel digests, and the adapter tree digest.
7. Verifies `vendor.json` is schema 2 with all four ABIs, each ABI directory has
   the vendored packages and `_pydantic_core*.pyd`, the detected interpreter's
   ABI is in the set, and at least one `TK*.dll` was staged.
8. `node scripts\desktop-pack.cjs --win nsis --x64 --publish never` —
   electron-builder copies the stage to `resources\backend\` and builds the
   NSIS installer. Publication is always forced to `never`; a stage without
   `vendor.json` is refused.
9. Prints the installer path, size and SHA-256, and the post-install smoke
   checklist.

Output: `studio\release\kala-studio-<version>-setup.exe`.

## What the package contains

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

FreeCAD itself is **not** bundled. The FreeCAD provider probes `FreeCADCmd` /
`freecadcmd` on `PATH` and reports the provider unavailable when FreeCAD is
missing. The adapter path is discovered from the executable's ancestors
(`resources\backend\adapters\freecad\src`), so the provider finds the bundled
modules without a repository checkout or a virtualenv. At runtime it probes
`FreeCADCmd`'s Python and adds only `src` plus the one matching
`site-packages-cpNNN` directory to `sys.path`; one installer therefore works
with FreeCAD builds on Python 3.11, 3.12, 3.13 or 3.14. An unsupported Python
is reported as unavailable with its version rather than crashing. The
`mod\Kala` workbench addon is staged for copying into FreeCAD's user `Mod`
directory; it is not auto-installed.

## Post-install smoke checklist

Run this on the Windows machine after installing:

1. Install as the current user (not elevated).
2. Set a throwaway data directory for the first launch:
   `set KALA_AGENT_DATA_DIR=%TEMP%\kala-studio-smoke-data` and
   `set KALA_STUDIO_DATA_DIR=%TEMP%\kala-studio-smoke-data`.
3. Launch Kala Studio. Task Manager must show exactly one `kala-studio.exe`
   child and no `python.exe`.
4. The window must load the workspace from `http://127.0.0.1:<port>/`.
5. Health: `curl.exe http://127.0.0.1:<port>/api/studio/health` → `status: ok`
   with `design_available: true`.
6. OpenCascade L-bracket: `volume_mm3 = 35571.416047` (rel 1e-6), bounds
   `[-30,30] × [0,45] × [0,40]`.
7. FreeCAD L-bracket (FreeCAD 1.1.x installed; not bundled): the provider must
   be available and the design must build in a real `FreeCADCmd` subprocess
   (`measured_by: FreeCAD`) with the same volume. The installed app supplies
   the adapter from `resources\backend\adapters\freecad\src` and the matching
   `site-packages-cpNNN` directory; do **not** add the repository virtualenv or
   a `PYTHONPATH` — no venv is needed. A FreeCAD build with Python 3.11-3.14 is
   supported; an unsupported Python appears as an unavailable reason naming its
   exact version.
8. STEP export and re-import: view bounds match within 0.001 mm.
9. Close the window: `kala-studio.exe` exits; no orphan process remains.
10. Uninstall: the user data directory is left in place.

## Notes and limits

* Cross-compilation from Linux with MinGW is a **check**, not a release build.
  OCCT is linked through MSVC import libraries, so the OCCT-linked crates are
  verified on Windows (this script) or by the `windows-check` cell for the
  non-OCCT crates. `x86_64-w64-mingw32-gcc` is not installed on the Linux
  packaging host.
* The installer is unsigned unless you configure `CSC_LINK`/`CSC_KEY_PASSWORD`
  (or a hardware token) for electron-builder. No certificate is read or
  committed by this script.
* NSIS downloads its toolchain into the electron-builder cache on first run.
* `--publish never` is enforced by `desktop-pack.cjs`; the script never uploads
  anything.
