# Linux AppImage packaging

Kala Studio's Linux deliverable is a single AppImage built by electron-builder.
The package launches **only** the native Rust `kala-studio`; there is no Python
interpreter, site-packages or PyInstaller payload in it. The only Python source
in the package is the FreeCAD adapter (`resources/backend/adapters/freecad/`),
which runs inside the user's own FreeCAD installation.

## What is staged

`studio/scripts/native-backend.cjs` builds and stages `studio/backend-native/`:

```
backend-native/
  kala-studio                 Rust release runtime
  studio_dist/                Vite UI served by the backend
  native-build.json           digests, source parity, staging metadata
  lib/                        exact ldd-resolved OCCT libTK*.so* (when linked)
  adapters/freecad/
    vendor.json               supported ABIs, wheel digests, detected FreeCAD
    src/kala_agent_freecad/   adapter modules for FreeCADCmd
    src/kala_agent/           shared pure-Python dependency (contracts/tools)
    site-packages-cp311/      vendored wheels for FreeCAD's Python 3.11
    site-packages-cp312/      vendored wheels for FreeCAD's Python 3.12
    site-packages-cp313/      vendored wheels for FreeCAD's Python 3.13
    site-packages-cp314/      vendored wheels for FreeCAD's Python 3.14
    mod/Kala/                 FreeCAD workbench addon
```

electron-builder copies that directory to `resources/backend/` unchanged.
`FreeCADCmd` is looked up on `PATH`; FreeCAD itself is never bundled. The
provider discovers the adapter by walking the executable's ancestors
(`resources/backend/adapters/freecad/src`). At runtime the Rust provider
probes `FreeCADCmd`'s own Python and hands `FreeCADCmd` exactly two `sys.path`
entries: `src` (adapter + shared `kala_agent`) and the one matching
`site-packages-cpNNN` directory. It never puts another ABI's compiled extension
on that path.

## FreeCAD Python dependencies (vendored for every ABI)

The adapter runs inside FreeCAD's interpreter, so its dependencies are vendored
for every CPython ABI current FreeCAD releases ship (`cp311`, `cp312`, `cp313`,
`cp314`) at stage time:

1. `freecadcmd <probe.py>` reports `sys.version_info`, `cache_tag`, `SOABI` and
   `sysconfig.get_platform()`; the stage records the build host's FreeCAD
   (here FreeCAD 1.1.4 / Python 3.14.8 / ABI `cp314`) for provenance and the
   in-FreeCAD verification.
2. For each ABI, `pip download --only-binary=:all: --python-version 3.X
   --implementation cp --abi cp3XX --platform manylinux2014_x86_64` resolves
   the pins in `studio/scripts/freecad-python-deps.json` (pydantic 2.13.5,
   pydantic-core 2.46.5, annotated-types, typing-extensions, typing-inspection).
3. Each wheel set is unpacked into its own
   `adapters/freecad/site-packages-cpNNN/`; the pure-Python `kala_agent` tree is
   staged once under `src/kala_agent`.
4. `FreeCADCmd` imports `pydantic`, `kala_agent.contracts`,
   `kala_agent.tools.protocol` and `kala_agent_freecad.worker` from `src` plus
   the build host's matching ABI directory before packaging; the result is
   recorded under `verification`.
5. Every wheel file name, byte count and SHA-256, every ABI's file count and
   dependency-tree digest, and the detected FreeCAD identity are written to
   `adapters/freecad/vendor.json` (schema 2) and mirrored into
   `native-build.json`. `desktop-pack.cjs` refuses a stage without `vendor.json`.
   `native-backend.cjs` refuses an ABI outside cp311-cp314.

Pass `--no-vendor-freecad` only for a throwaway stage (packaging then refuses
it). When FreeCAD is not installed on the build host, all four ABI sets are
still resolved and only the in-FreeCAD import verification is skipped
(`vendor.freecad.detected` is `false`). `--freecad-python-version`/`--freecad-abi`
restrict the set to one interpreter as a development aid, not for releases.

**One AppImage covers current FreeCAD Pythons.** This host's FreeCAD 1.1.4 uses
Python 3.14.8 (`cp314`), while the official FreeCAD 1.1.4 Linux AppImage ships
Python 3.11 (`py311`); the same AppImage works with both because the runtime
probe selects `site-packages-cp314` or `site-packages-cp311`. A FreeCAD build
with an unsupported Python (for example 3.10) is reported as
provider-unavailable with its exact version in the reason instead of crashing.

The `mod/Kala` workbench addon is staged for copying into FreeCAD's user `Mod`
directory; it is not auto-installed. See **Installing the FreeCAD add-on**
below for the one supported way to install it.

## Installing the FreeCAD add-on

The add-on is self-contained: it carries the adapter modules, the shared
`kala_agent` contracts package and the vendored dependencies for your
FreeCAD's Python ABI. It never reads a repository checkout and never uses a
virtualenv.

**The one supported install method: copy the staged add-on and its payload
into FreeCAD's user `Mod` directory.**

```bash
# 1. Extract the AppImage once (the AppDir of a mounted AppImage works too).
./kala-studio-0.1.0-x86_64.AppImage --appimage-extract
ADAPTER=./squashfs-root/resources/backend/adapters/freecad

# 2. Ask FreeCAD which Python it runs. The add-on needs that exact ABI:
#    cp311, cp312, cp313 or cp314.
ABI=$(FreeCADCmd -c 'import sys; print("cp%d%d" % sys.version_info[:2])')

# 3. Copy the add-on and its payload into FreeCAD's user Mod directory.
MOD="${XDG_DATA_HOME:-$HOME/.local/share}/FreeCAD"
[ -d "$MOD/v1-1" ] && MOD="$MOD/v1-1"
mkdir -p "$MOD/Mod/Kala"
cp -r "$ADAPTER/mod/Kala/." "$MOD/Mod/Kala/"
cp -r "$ADAPTER/src" "$MOD/Mod/Kala/src"
cp -r "$ADAPTER/site-packages-$ABI" "$MOD/Mod/Kala/site-packages-$ABI"
```

The result is a self-contained add-on:

```
~/.local/share/FreeCAD/Mod/Kala/
  Init.py  InitGui.py  kala_gui.py  kala_paths.py
  src/kala_agent_freecad/    adapter modules
  src/kala_agent/            shared contracts package
  site-packages-cpNNN/       the dependencies for your FreeCAD's Python
```

FreeCAD picks it up on the next start: the **Kala** workbench appears in the
workbench selector and the design box opens on the right.

Only the wheel set that matches your FreeCAD is copied (about 9 MiB); the
other three ABIs stay inside the AppImage.

**The developer symlink still works.** `python -m kala_agent_freecad.panel`
links `mod/Kala` into the same user `Mod` directory. Point it at the staged
tree (`studio/backend-native/adapters/freecad/`) when you want the
ABI-matched dependencies as well — the repository checkout itself has no
vendored wheel sets.

The add-on finds its payload in three shapes, in this order:

1. `KALA_FREECAD_SYS_PATH` — the directory that holds the adapter sources
   (the native provider sets it to `<adapter root>/src`);
2. the add-on's own folder — `src/` and `site-packages-<abi>/` next to it,
   which is the copied install above;
3. the adapter root one level above the add-on folder — which is the linked
   (symlinked) install.

If the payload is missing, the design box says so in its status line instead
of failing with a traceback.

### OCCT shared libraries

On Linux staging runs `ldd` on the staged binary and copies every resolved
`libTK*.so*` plus any non-system dependency that lives outside `/lib`,
`/lib64`, `/usr/lib` or `/usr/lib64`. The copied file uses the loader name
(`libTKernel.so.7`), not the fully versioned target name.

When `patchelf` is available (it is on the build host), the staged copy of
`kala-studio` gets `RUNPATH=$ORIGIN/lib` and each bundled library gets
`RUNPATH=$ORIGIN`. The source Cargo binary is never modified. If `patchelf` is
missing, staging keeps the build-time rpath and the Electron main process
prepends `resources/backend/lib` to `LD_LIBRARY_PATH`; the smoke test does the
same. `native-build.json` records `rpath`, `patchelf`, `sharedLibraryMode`,
and each library's sha256, so the choice is auditable.

## Build

```bash
export RUSTUP_HOME=/home/v9i/.rustup
export CARGO_HOME=/home/v9i/.cargo
(cd studio && npm ci)
packaging/linux/build-appimage.sh
```

The script runs `npm run build`, `npm run build:native`, then
`node scripts/desktop-pack.cjs --linux AppImage --x64 --publish never`, prints
the artifact path/size/sha256, and runs the headless smoke test. Use
`--no-smoke` to stop after the artifact.

The release gate is the current-feature native-design readiness report
(`studio/scripts/native-readiness.cjs`), not the old `nativeStep` feature flag.
When readiness is pending, only an effectively unpacked `--dir` artifact is
allowed. With readiness satisfied, the AppImage is a release artifact; the
report also states how many route-methods go through the geometry-backend seam
(the default build answers them natively; the Python worker is opt-in).

Output: `studio/release/kala-studio-<version>-x86_64.AppImage`.

## The built artefact (size and SHA-256)

The digest below belongs to the file **in this tree right now**. It
changes with **every** rebuild — the Rust backend is compiled from
sources other cells are still editing — so it is recorded here and
**only** here. Every other document points at this file instead of
quoting a digest that would go stale.

| | |
|---|---|
| File | `studio/release/kala-studio-0.1.0-x86_64.AppImage` |
| Size | **163 889 259 bytes** (156.3 MiB) |
| SHA-256 | `bf4734e18843fceb1d9bc98a305deddb5de771684f27772bd8c50dfa1d23149e` |
| Built | 2026-10-21, final build of the completeness wave; smoke `--require-geometry --require-freecad` PASS. Unsigned. |

Verify it with:

```bash
sha256sum studio/release/kala-studio-0.1.0-x86_64.AppImage
stat -c %s studio/release/kala-studio-0.1.0-x86_64.AppImage
```

The build script prints the same three numbers (path, size, SHA-256) after
it builds, so a user never has to trust a digest from a document.

## Smoke test

```bash
node packaging/linux/smoke-appimage.mjs \
  --appimage studio/release/kala-studio-0.1.0-x86_64.AppImage \
  --require-geometry --require-freecad
```

`xvfb-run` is not installed on this host and is not required: the script uses
`--appimage-extract` (no FUSE, no X server), then runs
`resources/backend/kala-studio` directly in two phases, each with an explicit
child environment (nothing inherited):

1. `PATH=""` — proves the runtime does not need Python at all. Checks
   `/api/studio/health`, the packaged UI, a 401 for an unauthenticated preview,
   the OpenCascade L-bracket (`volume_mm3 = 35571.416047`, rel 1e-6, bounds
   `[-30,30] × [0,45] × [0,40]`), the HLR drawing (front/top/right), the XZ
   section at `y = 4` (area `2400 − 25π`, `area_exact = true`, 1 outer + 1
   inner loop) and the STEP export/import round trip (bounds within 0.001 mm,
   digest and solid count equal).
2. `PATH=<dir of FreeCADCmd>:/usr/bin:/bin:/usr/local/bin` with a temporary
   working directory — runs the FreeCAD provider from the extracted app. No
   repository, no virtualenv, no `PYTHONPATH`, no `KALA_FREECAD_SYS_PATH`: the
   adapter and its vendored dependencies come from
   `resources/backend/adapters/freecad/src`, and FreeCAD itself is the host
   installation. The provider must be available, build the L-bracket by
   `FreeCADCmd` (`measured_by = FreeCAD`, the same volume and bounds), and pass
   the same drawing, section and STEP import checks.

Both phases get a temporary `HOME`, `XDG_DATA_HOME`, `KALA_AGENT_DATA_DIR`,
`KALA_STUDIO_DATA_DIR` and working directory (USER DATA rule: the real
`~/.local/share/kala-agent` is never touched). `LD_LIBRARY_PATH` is supplied
only if the staged binary has no `$ORIGIN/lib` RPATH; with `patchelf` it is
deliberately omitted so the RPATH itself is exercised.

Run the whole smoke under `env -i` for the cutover acceptance:

```bash
env -i PATH=/usr/bin:/bin HOME=$(mktemp -d) TMPDIR=/tmp \
  /path/to/node packaging/linux/smoke-appimage.mjs --require-geometry --require-freecad
```

## Notes and limits

* The AppImage is unsigned; it is an internal delivery artifact, not a store
  release.
* Electron still needs a display for the shell itself. The extracted-backend
  smoke path verifies the packaged runtime, not Chromium window rendering.
* Solid Edge is Windows-only and is not reachable from Linux.
