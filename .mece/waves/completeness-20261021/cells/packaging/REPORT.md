# Packaging cell report — backend log, FreeCAD add-on, desktop identity, app icon

Cell: `packaging` · Wave: `completeness-20261021` · Date: 2026-10-21
Brief: `cells/packaging/TASK.md` · Rules: `../../WAVE.md` · Audit: `../audit-backend/AUDIT.md` (B1, B5, B6, B7)

No commit was made. No file outside the ownership list was edited.

## Files

| Path | Status | What it carries |
|---|---|---|
| `studio/electron/backend-log.cjs` | new | The bounded backend log (`RotatingLog`), the secret scrubber and the startup-failure body builder. |
| `studio/electron/main.cjs` | modified | Pipes the backend child's stdout/stderr into the log; reports every startup failure with `dialog.showErrorBox`; exports the new helpers. |
| `studio/electron/main.d.cts` | modified | Type declarations for the new exports. |
| `studio/test/backendLog.test.ts` | new | 8 tests: rotation, token scrubbing, the dialog path, the child-output capture. |
| `adapters/freecad/mod/Kala/kala_paths.py` | new | Where the adapter and its ABI-matched dependencies live. No `parents[4]`, no `.venv`. |
| `adapters/freecad/mod/Kala/Init.py` | modified | Prepares the paths in every session, console sessions included. |
| `adapters/freecad/mod/Kala/InitGui.py` | modified | Prepares the paths and loads the dock; no hard-coded user `Mod` path any more. |
| `adapters/freecad/mod/Kala/kala_gui.py` | modified | Uses `kala_paths`; a missing payload is reported in the status line instead of a traceback. |
| `adapters/freecad/tests/test_addon_paths.py` | new | 6 tests for the resolver (no FreeCAD needed). |
| `studio/scripts/native-backend.cjs` | modified | Stages `kala_paths.py`, verifies the staged add-on inside `FreeCADCmd`, prunes the bytecode the check leaves behind, records the result in `native-build.json`. |
| `studio/scripts/native-backend.d.cts` | modified | Type declarations for the new staging result and the verification helper. |
| `studio/electron-builder.json` | modified | `syncDesktopName`, `StartupWMClass`, the icon, the description, `electron/backend-log.cjs` in `files`. |
| `studio/package.json` | modified | `"desktopName": "kala-studio.desktop"` (the only field this cell changed). |
| `studio/electron/icon.svg` | new | The icon source (Prussian-blue drafting mark on vellum). |
| `studio/electron/icon.png` | new | 512 × 512 PNG. |
| `studio/electron/icon-256.png` | new | 256 × 256 PNG. |
| `studio/electron/icon.ico` | new | Windows icon, 16/32/48/64/128/256. |
| `packaging/linux/README.md` | modified | The add-on install steps (this cell's section of the file only). |

The icon assets live in `studio/electron/` (the build-resources directory) and
not in `studio/build/`, because the repository `.gitignore` ignores every
`build/` directory — assets there would never be tracked.

---

## B1 — backend log and startup failure dialog

### Change

`studio/electron/backend-log.cjs` is a new module with three pieces:

* **`RotatingLog`** — appends the child's output to `<logs>/backend.log`
  (Electron's `app.getPath('logs')`, i.e. `~/.config/kala-studio/logs/` on
  Linux). The current file plus four archives, each at most 1 MiB, so the log
  can never grow past ~5 MiB. The file is created with mode 0600.
* **`scrubSecrets`** — removes the bearer token (the literal value) and
  anything shaped like a token (`KALA_STUDIO_TOKEN=…`, `Authorization: Bearer
  …`) before a chunk is written.
* **`formatStartupFailure` / `reportStartupFailure`** — build the plain-text
  body of the startup dialog: what failed, where the log is and the last 20
  log lines, then call `dialog.showErrorBox`.

`studio/electron/main.cjs` now:

* creates the log before anything can fail (`createBackendLogger({ token })`),
  so every failure below has a log to point at;
* pipes `child.stdout` and `child.stderr` into it (`BackendProcess.writeLog`);
* reports the **missing binary**, the **spawn error / early exit** and the
  **health timeout** with `reportStartupFailure(...)` before `app.exit(1)`,
  and the **unexpected exit after readiness** with the same body before
  `app.quit()`.

The Studio is the only writer: the backend is never told where the log is,
never writes it and never receives a path to it.

### Evidence — the backend never prints prompts or keys

Every `eprintln!`/`println!` in the native crate was read: they carry error
kinds and names only, never prompt text, key material or the token
(`backend/crates/kala-studio/src/boundary.rs:418`, `server.rs:625,1076,1171,
1237,1305,1456,1551,1611,1629,1684`, `cli.rs:111,126,134,156,170,177,195,205,
219,295`, `native_geometry/mod.rs:1132`). The journal keeps `prompt_sha256`
and `prompt_chars`, never the prompt (audit "Verified working").

### Evidence — the real Electron app, end to end

Run against the development override with a missing interpreter (a temporary
`XDG_CONFIG_HOME`, nothing of the user was touched):

```
$ KALA_STUDIO_PYTHON=/nonexistent/python3 electron .
[kala-studio] Kala Studio could not start
the development-only Python reference backend is not executable at /nonexistent/python3. …

The backend program was not found, so the workspace could not open.

Log file: /tmp/kala-packaging-cell/xdg/config/kala-studio/logs/backend.log

Last 1 log line:
[kala-studio] the backend binary is missing at /nonexistent/python3
EXIT=1
```

With a backend that exits immediately (`/bin/false`):

```
[kala-studio] Kala Studio could not start
The Kala Studio backend did not become ready, so the workspace could not open.

backend exited before reporting health (exit=1 signal=null)

Log file: …/kala-studio/logs/backend.log

Last 1 log line:
[kala-studio] failed to start backend: backend exited before reporting health (exit=1 signal=null)
EXIT=1
```

With a backend that writes to stdout and stderr before exiting, both streams
land in the log and both are shown in the dialog:

```
Last 3 log lines:
hello from the fake backend stdout
hello from the fake backend stderr
[kala-studio] failed to start backend: backend exited before reporting health (exit=3 signal=null)
```

(The headless machine has no display, so `dialog.showErrorBox` itself cannot
open a window; the failure is caught and the same body is written to stderr.
On a normal desktop the modal box carries exactly this text.)

### Evidence — the new test file

`studio/test/backendLog.test.ts` (8 tests, all green):

| Test | What it pins |
|---|---|
| `rotates instead of growing without limit` | 40 writes → at most 5 files, each ≤ 1 MiB, newest lines survive |
| `keeps the last lines in memory for the error dialog` | `tail(20)` returns the last 20 lines |
| `never writes the bearer token, even when the backend echoes it` | the token and a `Bearer` value never reach the file or the tail |
| `scrubs an environment assignment that names the token` | `KALA_STUDIO_TOKEN=…` is redacted |
| `captures the backend child output and scrubs the token out of it` | a real child process is spawned; its stdout reaches the log, the token does not |
| `shows the reason, the log path and the last log lines` | the dialog body carries the reason, the log path and the last lines — and never the token |
| `survives a machine that cannot show a dialog` | a throwing dialog does not take the app down |
| `puts the log file under the Electron logs directory` | `app.getPath('logs')` is used, with a fallback |

---

## B5 — a self-contained FreeCAD add-on

### Change

**`adapters/freecad/mod/Kala/kala_paths.py`** (new) resolves, in this order:

1. `KALA_FREECAD_SYS_PATH` — every entry on `os.pathsep` is probed, and the
   directory next to it is probed for the ABI wheel set (this is what the Rust
   provider sets: `<adapter root>/src`);
2. the add-on's own folder — `src/` and `site-packages-<abi>/` beside it (the
   copied install);
3. the add-on folder's `payload/` sub-folder;
4. the adapter root one level above the add-on folder — `<root>/mod/Kala` →
   `<root>` (the linked install).

The ABI directory name comes from the running interpreter:
`sys.implementation.cache_tag` (`cpython-314`) → `cp314`. Only the matching
wheel set is ever put on `sys.path`.

The shared `kala_agent` contracts package is found by looking for the
directory that **actually contains it** (`src/kala_agent/__init__.py`), never
by counting parents to guess a repository root. `parents[4]` and the
repository `.venv` are gone — the test
`test_the_addon_module_has_no_repository_or_virtualenv_guess` pins that.

`Init.py` now prepares the paths (FreeCAD runs it in every session, console
sessions included — it is the reason a console session can import the
adapter). `InitGui.py` prepares them again and imports the dock; it no longer
hard-codes `FreeCAD.getUserAppDataDir()/Mod/Kala`. `kala_gui.py` uses
`kala_paths` and, when the payload is missing, says so in its status line
instead of raising an import error into the workbench.

**Staging** (`studio/scripts/native-backend.cjs`):

* `mod/Kala/kala_paths.py` is now a required staged file;
* `verifyStagedFreeCadAddOn()` imports the staged add-on **inside
  FreeCADCmd**: only the add-on folder is put on `sys.path`, the add-on
  resolves the rest by itself, exactly as FreeCAD does when it loads the
  add-on from the user's `Mod` directory. A stage that cannot do that is
  refused (and the check is mandatory with `--require-freecad-vendor`);
* the check leaves bytecode caches behind (FreeCAD's embedded interpreter
  ignores `PYTHONDONTWRITEBYTECODE`), so the stage is pruned afterwards — a
  stage carries sources, never bytecode;
* the result is recorded in `native-build.json` under
  `freeCadAdapter.addonVerification`.

### The one install method (documented in `packaging/linux/README.md`)

**Copy the staged add-on and its payload into FreeCAD's user `Mod`
directory.**

```bash
./kala-studio-0.1.0-x86_64.AppImage --appimage-extract
ADAPTER=./squashfs-root/resources/backend/adapters/freecad
ABI=$(FreeCADCmd -c 'import sys; print("cp%d%d" % sys.version_info[:2])')
MOD="${XDG_DATA_HOME:-$HOME/.local/share}/FreeCAD"
[ -d "$MOD/v1-1" ] && MOD="$MOD/v1-1"
mkdir -p "$MOD/Mod/Kala"
cp -r "$ADAPTER/mod/Kala/." "$MOD/Mod/Kala/"
cp -r "$ADAPTER/src" "$MOD/Mod/Kala/src"
cp -r "$ADAPTER/site-packages-$ABI" "$MOD/Mod/Kala/site-packages-$ABI"
```

Only the wheel set that matches the user's FreeCAD is copied (~9 MiB); the
other three ABIs stay inside the AppImage. The **developer symlink keeps
working**: `python -m kala_agent_freecad.panel` links `mod/Kala` into the
same directory, and the resolver finds the payload one level above the
add-on folder (point it at the staged tree to get the ABI-matched wheels as
well — the repository checkout has none).

### Evidence — `FreeCADCmd` in a temporary user home (nothing of the user touched)

`FREECAD_USER_HOME` is a temporary directory in every run; the real
`~/.local/share/FreeCAD` was never written.

```
--- A: copied into a TEMP FreeCAD user dir (the documented install) ---
KALA_CHECK {"cacheTag": "cpython-314", "state": {"abi": "cp314",
  "adapter": "…/homeA/Mod/Kala/src", "shared": "…/homeA/Mod/Kala/src",
  "sitePackages": "…/homeA/Mod/Kala/site-packages-cp314", "added": []},
  "problems": [],
  "models": "…/homeA/Mod/Kala/src/kala_agent_freecad/cad/models.py",
  "pydantic": "2.13.5"}

--- B: linked to the staged add-on (the developer symlink) ---
KALA_CHECK {"cacheTag": "cpython-314", "state": {"abi": "cp314",
  "adapter": "…/squashfs-root/resources/backend/adapters/freecad/src",
  "sitePackages": "…/squashfs-root/resources/backend/adapters/freecad/site-packages-cp314"},
  "problems": [], "pydantic": "2.13.5"}

--- C: the add-on without its payload (must report the missing payload) ---
KALA_CHECK {"state": {"adapter": null, "sitePackages": null},
  "describe": "The Kala adapter was not found. Install the add-on from the AppImage or set KALA_FREECAD_SYS_PATH.",
  "problems": ["the Kala adapter sources were not found next to the add-on …",
               "the vendored dependencies for cp314 were not found next to the add-on …"],
  "models_error": "ModuleNotFoundError(\"No module named 'kala_agent_freecad'\")"}

--- D: KALA_FREECAD_SYS_PATH points at the staged adapter sources ---
KALA_CHECK {"state": {"abi": "cp314", "adapter": "…/adapters/freecad/src",
  "sitePackages": "…/adapters/freecad/site-packages-cp314"}, "problems": [],
  "pydantic": "2.13.5"}
```

The add-on was taken from the **extracted AppImage** (`…/squashfs-root/resources/
backend/adapters/freecad`), i.e. from the very tree a user installs from —
not from the repository stage, so nothing in the tree was polluted.

Shape C is the important one: without the payload the add-on now **says so**
instead of failing with an import error.

The same check runs inside the staging itself, on every build:

```
[native-backend] the staged FreeCAD add-on was verified inside FreeCADCmd (15 bytecode caches removed)
```

### Evidence — the new test file

`adapters/freecad/tests/test_addon_paths.py` (6 tests, all green): the
resolver has no `parents[4]` and no `.venv`; the ABI tag follows
`sys.implementation.cache_tag`; `KALA_FREECAD_SYS_PATH` is honoured; only the
matching ABI directory is put on `sys.path`; a missing payload is reported
honestly; the summary names the adapter and the ABI.

---

## B6 — desktop identity

### Change

* `studio/package.json`: `"desktopName": "kala-studio.desktop"`.
* `studio/electron-builder.json`: `linux.syncDesktopName: true`,
  `linux.desktop.entry.StartupWMClass: "kala-studio"`,
  `linux.synopsis` / `linux.description` (the desktop `Comment`),
  `linux.category` (`Engineering`), `electron/backend-log.cjs` added to
  `files`.

electron-builder overwrites `Categories` from `linux.category` and `Comment`
from `linux.description`, so the dead `Categories` line was removed from the
desktop entry instead of leaving a config that never reaches the file.

### Evidence — the generated `.desktop` (extracted from the AppImage)

```
[Desktop Entry]
Name=Kala Studio
Exec=AppRun --no-sandbox %U
Terminal=false
Type=Application
Icon=kala-studio
StartupWMClass=kala-studio
X-AppImage-Version=0.1.0
Comment=Kala Studio — a standalone parametric mechanical design workspace: agent chat, drafting sheet, STEP/STL export and FreeCAD.
Keywords=cad;mechanical;design;fea;stl;step
Categories=Engineering;
```

The file name is `kala-studio.desktop` (from `desktopName` +
`syncDesktopName`), the `WM_CLASS` is `kala-studio` (the executable name),
and the `Icon` is `kala-studio` — the icon that is actually installed.

---

## B7 — app icon

### Change

`studio/electron/icon.svg` is the source: a vellum sheet (`#F6F3EB`) with a
drafted hairline frame (`#C8C2B6`), a construction line (`#DFDBD0`) and the K
of Kala in Prussian blue (`#2B5B84`) — the Vellum Drafting Board language of
the Studio, drawn with the same line weights as the workspace. The strokes
are heavy on purpose (64 of 512 units) so the mark still reads at 16 px,
where the hairline frame and the construction line fall away on their own.

Generated with the converters that exist on this host:

```
rsvg-convert -w 512 -h 512 icon.svg -o icon.png
rsvg-convert -w 256 -h 256 icon.svg -o icon-256.png
magick icon.png -define icon:auto-resize=256,128,64,48,32,16 icon.ico
```

(`rsvg-convert` → PNG, ImageMagick `magick` → ICO.)

`electron-builder.json` points `linux.icon` at `icon.png` and `win.icon` at
`icon.ico`.

### Evidence

```
$ identify icon.png icon-256.png
icon.png PNG 512x512 512x512+0+0 8-bit sRGB 18601B
icon-256.png PNG 256x256 256x256+0+0 8-bit sRGB 8370B

$ identify icon.ico
icon.ico[0] ICO 256x256 … icon.ico[1] ICO 128x128 … icon.ico[2] ICO 64x64
icon.ico[3] ICO 48x48 … icon.ico[4] ICO 32x32 … icon.ico[5] ICO 16x16
```

Inside the AppImage (the stock Electron icon is gone — the old one had the
sha256 of `app-builder-lib/templates/icons/electron-linux/256x256.png`):

```
$ sha256sum usr/share/icons/hicolor/512x512/apps/kala-studio.png studio/electron/icon.png
8f5563ad0f46e19af2f4a4d36be7a312a4c4de97bd9bf73809ea90b652fae7f3  …/usr/share/icons/hicolor/512x512/apps/kala-studio.png
8f5563ad0f46e19af2f4a4d36be7a312a4c4de97bd9bf73809ea90b652fae7f3  …/studio/electron/icon.png
```

The 16 px rendering was checked by eye (a zoomed 16 px render still reads as
a K).

**The user can replace the icon**: edit `studio/electron/icon.svg` (or drop
another 512 px PNG over `studio/electron/icon.png`) and rebuild. The SVG
header carries the three commands.

---

## Gates

Every command ran on this Linux host with a temporary `HOME`,
`KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`
(`RUSTUP_HOME=/home/v9i/.rustup`, `CARGO_HOME=/home/v9i/.cargo`). The real
`~/.local/share/kala-agent/**` and the real `~/.local/share/FreeCAD` were
never written; no credential or settings file was read; no `claude`, `codex`
or `opencode` CLI was run; nothing was installed into `~/Applications`; no
commit was made.

| Gate | Result |
|---|---|
| `cd studio && npm run typecheck` | **pass** (no output) |
| `cd studio && npm test` | **pass** — 53 files, **758 tests** |
| `cd studio && npm run build` | **pass** — vite build in 462 ms |
| `adapters/freecad/tests` (pytest) | **pass** — **403 passed** (397 existing + 6 new) |
| `ruff check` on the owned Python | **All checks passed** |
| `mypy` on `kala_paths.py` | **Success: no issues found** |
| `packaging/linux/build-appimage.sh` | **pass** (Vite → native stage → electron-builder → smoke) |
| `node packaging/linux/smoke-appimage.mjs --appimage studio/release/kala-studio-0.1.0-x86_64.AppImage --require-geometry --require-freecad` | **PASS with FreeCAD provider** — OCCT L-bracket 35571.416047 mm³, FreeCAD L-bracket 35571.416047 mm³, drawing front(7)/top(11)/right(8), XZ@4 section 2321.4601836602546 mm² (exact, 1 outer + 1 inner loop), STEP round trip 1 solid |
| `df -h /tmp` before the build | 5.2 GB free (≥ 3 GB required); `/tmp` cleaned afterwards (4.3 MB left in the cell's own temp dir, the AppImage extractions removed) |

### AppImage

| | |
|---|---|
| File | `studio/release/kala-studio-0.1.0-x86_64.AppImage` |
| Size | **163 868 354 bytes** (156.3 MiB) |
| SHA-256 | **`eed6ef7b780fe1a3372e9e86e3e3f8d6b74b74395c2f5469618125970ea0ff98`** |

(The digest changes with every build — the backend is rebuilt from sources
that other cells are still editing. The value above is the one this cell
produced; the docs' digest table (audit B9) has to be regenerated from the
final build.)

### Concurrency note

The first two AppImage builds failed because the Rust backend did not
compile: the design-budget and runtime-fixes cells were editing
`server.rs` and `cli_runtime.rs` at that moment (`E0277: the size for values
of type [CliProbe] cannot be known at compilation time`). The build was
retried; the third attempt compiled and passed. Nothing was changed in the
Rust backend by this cell.

One staging failure was caused by bytecode caches inside the old stage: the
audit cell's own FreeCADCmd experiment had imported the adapter from
`studio/backend-native/`, and FreeCAD's embedded interpreter ignores
`PYTHONDONTWRITEBYTECODE`. The caches were removed and the staging now
prunes them after its own check.

The same trap exists for everyone else: **any** `FreeCADCmd` run that imports
the adapter out of `studio/backend-native/` (a manual test, the audit, a
developer) writes bytecode into the stage, and the next staging refuses the
polluted stage. The staging helper prunes its own check, but it cannot prune
a stage that was polluted from the outside. Running the add-on against the
*extracted AppImage* instead (what this cell's proof does) avoids it.

---

## Gaps and follow-ups

1. **The install doc still names the old log directory.**
   `docs/studio-install.md` points at `~/.local/share/kala-studio/log/`. The
   log now lives in Electron's logs directory (`~/.config/kala-studio/logs/`
   on Linux, `%APPDATA%\kala-studio\logs\` on Windows). `docs/**` is not in
   this cell's ownership — the docs cell has to correct it.
2. **The AppImage digest in the docs is stale** (audit B9) and stays stale
   until the final build; the value above is this cell's build.
3. **The Windows `.ico` was built but not opened on Windows** — there is no
   Windows host here. The file carries the six usual sizes.
4. **The dialog was not seen on a desktop.** This host has no display and no
   `xvfb-run`; the dialog body was verified through the same code path in a
   headless Electron run and through the test, not as a rendered window.
5. **The repository checkout has no vendored wheel sets**, so the developer
   symlink (which points at `<repo>/adapters/freecad/mod/Kala`) finds the
   adapter and the shared package but not the ABI-matched dependencies; the
   design box says so instead of failing. Pointing the symlink at the staged
   tree (`studio/backend-native/adapters/freecad/`) makes it complete — this
   is documented in `packaging/linux/README.md`. Moving the vendored wheels
   into the repository is a packaging decision that was not taken here.
6. **`adapters/freecad/src/kala_agent_freecad/panel.py` still hard-codes
   `~/.local/share/FreeCAD[/v1-1]/Mod`** and knows nothing about the payload.
   It lives in `adapters/freecad/src/**`, which this cell does not own; the
   add-on works around it, but the installer helper should learn about the
   payload in a later pass.
