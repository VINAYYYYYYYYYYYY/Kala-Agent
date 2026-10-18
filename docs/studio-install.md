# Installing and launching Kala Studio

> Scope: end-user installation, developer workflow, and CI artefacts.
> This document describes how the wave builds the desktop bundle. It
> does **not** edit the user's system configuration; running the
> launcher creates files only inside the application data directory
> and the desktop bundle's own resources.

## Two ways to run the studio

There are two supported ways to run Kala Studio. Both use the same
backend module; they differ only in how the backend is launched and
how the bearer token is supplied.

### 1. Standalone desktop bundle (Windows / Linux AppImage)

The unsigned desktop bundle is the consumer-facing entry point. It is
built by `.github/workflows/studio.yml` on Ubuntu and Windows runners
and uploaded as a build artefact.

On Windows the build produces an unsigned NSIS installer named
`kala-studio-<version>-setup.exe` (user-facing). The installer embeds
an unsigned `.nsis.7z` payload, which is an internal archive and
**not** an installer to run directly. The desktop shell launches
`resources/backend/kala-studio.exe`. The user must accept the
publisher warning ("Unknown publisher") because the build is not
code-signed.

On Linux the build produces an unsigned AppImage in
`studio/release/` named `kala-studio-<version>-<arch>.AppImage`
(e.g. `kala-studio-0.1.0-x86_64.AppImage`). The user runs it with a
normal double-click or on the command line. No system-level
installation is performed; the AppImage runs in its own filesystem
mount.

The first launch picks a free loopback port, generates a random
`KALA_STUDIO_TOKEN`, writes it to a launcher file with restricted
permissions (0600 on POSIX), starts the bundled backend, waits up to
15 s for `/api/studio/health` to return `{status:"ok"}`, and opens a
desktop window pointing at the loopback URL. The bearer token is
injected into the window via a single Electron IPC channel.

### 2. Developer mode (running the backend locally)

Developers building the UI separately can run the backend directly:

```bash
# from a clean checkout
python -m venv .venv && source .venv/bin/activate
pip install -e ".[studio]"
# Optional but recommended for real STEP export. Without it
# the /api/studio/export route for STEP returns 503.
pip install -e ".[geometry]"

# in one terminal. KALA_STUDIO_PYTHON is the convention the
# Electron shell uses to point at the project's venv; the same
# variable works for ad-hoc dev launches.
KALA_STUDIO_PYTHON="$PWD/.venv/bin/python" \
  kala-studio --host 127.0.0.1 --port 8765
# equivalent:
KALA_STUDIO_PYTHON="$PWD/.venv/bin/python" \
  python -m kala_agent.studio --host 127.0.0.1 --port 8765

# in a second terminal, build the UI once
cd studio
npm ci
npm run build
# Vite output lands in studio/dist and is served by the backend
npm run dev   # or open http://127.0.0.1:8765 in a browser
```

Without a token argument, the backend prints a randomly generated
`KALA_STUDIO_TOKEN` to the terminal. Copy it once per session and
paste it into the browser fallback prompt. The backend never returns
the token over HTTP and never logs it.

### 3. Browser fallback (no desktop bundle)

For headless or browser-only use:

1. Run `kala-studio` in a terminal.
2. Note the printed token.
3. Open `http://127.0.0.1:8765/` in a same-origin browser tab.
4. Paste the token into the UI's "Manual token" field. The token is
   held in `sessionStorage` only; closing the tab discards it.

Same-origin browser fallback does not bypass the loopback check; the
backend still rejects cross-origin requests.

## Windows launching

The wave produces an unsigned wrapper. Steps on a clean Windows 10 /
11 host:

1. Download the unsigned `kala-studio-<version>-setup.exe` from the
   CI run page.
2. Right-click it -> Properties -> tick "Unblock" if SmartScreen
   flagged the download.
3. Run the installer and accept the publisher warning ("Unknown
   publisher"). The build is unsigned; the warning is expected.
4. Right-click and "Run as administrator" is **not** required and is
   **not** recommended. The app runs in user mode.
5. The window opens to the studio workspace.
6. Close the window to shut the backend down. Confirm the python
   process has exited in Task Manager.

No Windows registry keys are written. No Windows services are
installed. No scheduled tasks are created. The wrapper does not
modify `PATH` or any shell startup file.

## Linux / Omarchy launching

On Linux the build is an AppImage. Steps:

1. `chmod +x kala-studio-<version>-<arch>.AppImage`
2. Run it:
   `./kala-studio-<version>-<arch>.AppImage`
3. If the host uses FUSE 3 and the AppImage fails to mount, use
   `--appimage-extract-and-run`. The wave has not exhaustively tested
   every desktop integration (GNOME, KDE, Hyprland, Sway); the
   AppImage is the only delivery format this wave ships.
4. The app writes its data directory under
   `$XDG_DATA_HOME/kala-studio/` (default
   `~/.local/share/kala-studio/`). The bearer token file is created
   with mode 0600.

The wave does **not** ship a `.desktop` file, an icon, or any
integration with the host's autostart / launcher menu. The user can
add such entries by hand; the wave does not edit the host's
configuration. On Omarchy (an opinionated Hyprland distribution) the
AppImage is expected to launch without affecting window rules or the
bar; this has not been verified on a clean host in this wave.

## Dev commands (studio/)

The Electron + React workspace owns its own package.json. Verify the
exact scripts in `studio/package.json`; the names below are the
ones this wave assumes.

```bash
cd studio
npm ci                # install deps, reproducible lockfile
npm run typecheck     # TypeScript strict mode over studio/src
npm run build         # Vite production build into studio/dist
npm run desktop       # launch electron + backend in dev mode
npm run desktop:pack  # electron-builder; reads studio/backend/kala-studio[.exe]
```

The build pipeline copies the PyInstaller backend binary into
`studio/backend/` before `npm run desktop:pack`. Do **not** check
that binary into version control; the desktop bundle is rebuilt
every release.

## Frozen backend smoke

After the PyInstaller binary exists at `dist/kala-studio`, the
repository ships a stdlib-only validator that proves the frozen
runtime starts and serves without any installed source tree:

```bash
python packaging/studio_smoke.py --backend dist/kala-studio
```

It launches the binary on a free loopback port, waits for a bounded
health deadline, checks the UI root and its relative assets,
requests a preview mesh, and exports a real STEP file when the
health probe advertises `step_available`. It uses a synthetic token
and a temporary working directory, never calls a live provider or
solver, reaps the child in a `finally` block, and exits non-zero on
any failure. `--allow-missing-step` relaxes the STEP requirement
only when the `[geometry]` writer is genuinely absent; CI never
passes it.

The CI matrix installs `.[dev,studio,geometry]`, so the geometry
install must succeed; it is not an optional or `continue-on-error`
step. The matrix runs the studio pytest suite and the UI
`npm test` suite, then the frozen-binary smoke above.

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

If a future build needs system-level integration, it must be
documented in this file with explicit user consent.

## Troubleshooting checklist

| Symptom | First thing to check |
---|---|
 Window opens then immediately closes | Backend did not respond to `/api/studio/health` within 15 s. Check the launcher log file in `~/.local/share/kala-studio/log/` or `%APPDATA%\kala-studio\log\`. |
 401 on every request | Token mismatch between Electron IPC and the backend. Restart the desktop; the token is regenerated each launch. |
 503 on STEP export | `cadquery-ocp` is not installed. Install it in the venv or rebuild the backend with the `[studio]` extra. The error message names the missing package. |
 Browser fallback prompts for token but the token was lost | Restart the backend and copy the freshly printed token. The backend never stores the token in any persistent log. |
 AppImage fails to mount on a host without FUSE | Use `--appimage-extract-and-run` once; the bundle still works. |
 CI artefact is missing on the GitHub Actions run | The matrix is gated on `ubuntu-latest` and `windows-latest`. Inspect the failed job's log; the wave does not retry or fallback to a runner that is not on the matrix. |
