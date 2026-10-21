# Cell audit-backend (READ-ONLY): what necessary things are missing in the backend, packaging, add-on and docs

Read `../../WAVE.md` first (the rules and the in-flight exclusions). You may read anything and run tests and builds in temp dirs. **Do not edit any source file.** Write only `AUDIT.md` in this folder.

## Check, with evidence
- **API vs the UI's real needs:** can the client download the provider's artifacts (FCStd, STEP) by digest? Is there a route, a bounded size and correct content types? Does a client abort or disconnect during a long chat or design turn cancel the work (the remote HTTP call, the CLI subprocess and its process group, the FreeCAD build), or keep running? Prove it either way with a test or an experiment. Is there an API-key "test" or validation path? Do health, providers and readiness describe the native reality (the packaging readiness report still says 10 route-methods are "delegated" or "proxied" though the native backend answers them; see `studio/scripts/native-readiness.cjs` and `desktop-pack.cjs` output)? Do the error envelopes stay consistent across the new code?
- **Data hygiene:** does the artifact store and the journal grow without bound? Is there retention or a documented limit? Are CLI private workdirs and FreeCAD temp files always removed (also on timeout or kill)? Logs: where does a user find backend logs when something fails in the packaged app?
- **Packaging (Linux and Windows definitions):** electron-builder printed "default Electron icon is used (application icon is not set)" and "Set desktopName in package.json and linux.syncDesktopName" (so Wayland/Hyprland cannot link the window to its .desktop entry). Check the icon, desktopName/StartupWMClass, the .desktop entry, the version and about info, and whether `packaging/windows/build-windows.ps1` and `.github/workflows/studio-release.yml` still match what Linux ships after this wave (CLI providers, FreeCAD vendor ABIs, smoke flags).
- **FreeCAD add-on** (`adapters/freecad/mod/Kala/kala_gui.py` `_prepare_path`): it only works via a symlink into the repo (it uses `parents[4]` and the repo `.venv`). Is it shipped or installable from the AppImage? What would a self-contained add-on need?
- **Docs drift:** `docs/studio-providers.md`, `studio-limitations.md`, `cad-providers.md`, `studio-install.md` and `studio.md` against today's behaviour: the 3 CLI providers, the 30 s chat / 180 s design budgets, the validation and build repair rounds, `cuts_effective`, the plane rule, the per-pocket warnings, the AppImage digest and install steps. List every stale statement.
- **Security re-check of the new surface** (CLI providers: env allowlist, argv, working dir, consent; the model catalog route; journal payloads free of secrets and prompt text).

## AUDIT.md format
A table, one row per finding: `id | area | finding | evidence (file:line, test, or experiment output) | user impact | severity (MUST / SHOULD / NICE) | suggested fix (files to touch) | needs user decision?`.
- MUST means a user hits it in normal use, or it is broken, misleading, unsafe or blocks install. Be strict: no padding.
- End with a summary: counts per severity, plus the MUST list in priority order.
Reply with that summary.
