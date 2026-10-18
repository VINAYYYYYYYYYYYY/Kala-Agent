# Delivery cell report

Cell: `delivery` (docs, CI, build pipeline, frozen-runtime smoke).
Wave: `studio-20261018`.
Briefs: `cells/delivery/FINISH-CI.md`, `cells/delivery/FINISH.md`,
`cells/delivery/RESUME-FREE.md`.
Shared contract: `../WAVE.md`. Review notes: `../REVIEW-NOTES.md`.
Sources: `../SOURCES.md`, `../OCP-VERIFIED.md`.

## Scope and ownership

Delivery owns the paths below only. No core, backend, UI, or
`src/kala_agent/studio/**` file is modified by this cell. Delivery did
**not** rebuild the PyInstaller binary; the conductor owns the final
source build (see "Frozen runtime" below).

| Path | Status | Notes |
|---|---|---|
| `pyproject.toml` | modified | `[studio] = [uvicorn]`; `[geometry] = [cadquery-ocp>=7.7]`; `kala-studio = kala_agent.studio.__main__:main`. |
| `.gitignore` | modified | appends PyInstaller, electron-builder, and packaging scratch artefacts. |
| `README.md` | rewritten | truthful honesty disclaimer (see below); quick start installs `.[studio]` then `.[geometry]`. |
| `docs/studio.md` | rewritten | contract, honest capabilities, no winding-defect warning, Codex fails closed, OpenCascade.js future vs native OCP now. |
| `docs/studio-install.md` | rewritten | install/dev/desktop paths, frozen-backend smoke section, and CI description; artefact names corrected to the real electron-builder output (`kala-studio-<version>-<arch>.AppImage`, NSIS installer `kala-studio-<version>-setup.exe`, `.nsis.7z` is a payload not an installer); broken numbered lists fixed. |
| `docs/studio-providers.md` | rewritten | `codex` documented as fail-closed / not implemented; OpenAI and OpenRouter are the only real API routes. |
| `docs/studio-limitations.md` | rewritten | native CAD/solver automation absent; five analysis targets representative; Windows runtime unverified; AppImage config is not proof of a working desktop. |
| `packaging/studio_backend.spec` | unchanged this pass | carries the conductor-verified `module_collection_mode` fix and `collect_submodules("kala_agent")`. |
| `packaging/studio_smoke.py` | new | stdlib-only frozen-runtime validator. |
| `.github/workflows/studio.yml` | rewritten | real 3-job matrix; no self-scanning secret step, no pseudo-job, no `continue-on-error`. |

## Actual checks

All commands below were run on this Linux host. Results are verbatim.

| Check | Result | Command |
|---|---|---|
| Root suite (core baseline + studio) | **1438 passed**, 1 warning | `.venv/bin/python -m pytest -q` |
| Studio suite only (all 9 studio files) | **168 passed**, 1 warning | `.venv/bin/python -m pytest -q tests/security/test_studio*.py tests/unit/test_studio*.py` |
| Frozen-runtime smoke, existing binary | **PASS** (exit 0) | `.venv/bin/python packaging/studio_smoke.py --backend dist/kala-studio --timeout 90` |
| Smoke fail-closed, missing binary | exit 1 with actionable message | `... studio_smoke.py --backend dist/does-not-exist` |
| `ruff check` owned Python | All checks passed | `.venv/bin/ruff check packaging/` |
| `ruff format --check` owned Python | 2 files already formatted | `.venv/bin/ruff format --check packaging/` |
| Strict `mypy` on the smoke file | Success, no issues | `.venv/bin/mypy --strict packaging/studio_smoke.py` |
| Studio ruff check (source + tests) | All checks passed | `.venv/bin/python -m ruff check src/kala_agent/studio tests/security/test_studio*.py tests/unit/test_studio*.py` |
| Studio ruff format --check | 32 files already formatted | `.venv/bin/python -m ruff format --check src/kala_agent/studio tests/security/test_studio*.py tests/unit/test_studio*.py` |
| Studio strict mypy | Success, 23 source files | `.venv/bin/python -m mypy --strict src/kala_agent/studio` |
| `py_compile` owned files | OK | `.venv/bin/python -m py_compile packaging/studio_smoke.py packaging/studio_entry.py` |
| Workflow YAML parses | jobs `[build, core-tests-linux, adapter-tests-linux]` | `yaml.safe_load` (PyYAML 6.0.3) |
| Workflow has no `continue-on-error` / `secrets.` | absent | literal scan |
| Workflow line length | max 77 chars | literal scan |
| `yamllint` | only `missing document start "---"` (informational) | `yamllint .github/workflows/studio.yml` |

### Test attribution

The pre-wave core baseline was **1270 passed**. The studio work adds
**168** Studio tests (studios' security, geometry, providers, mocked
HTTP, registry, server, static, error, and STEP files). 1270 + 168 =
**1438 passed** for the whole root suite, so the two numbers are
reconciled rather than blended.

The preview geometry is independently verified in
`tests/unit/test_studio_geometry.py` (29 tests). An independent
signed-volume check reports outward winding for all three primitives:
box 24000 mm³ (exact), cylinder 9397.88584 mm³, sphere 4144.123939 mm³
(1.066 % below exact, tessellation). The obsolete winding-defect
warning was removed from every doc.

## Frozen runtime and conductor build

`dist/kala-studio` (ELF 64-bit LSB executable, 175,280,512 bytes,
built 14:52) was built by the **conductor**, not by this cell. Build
log: `/tmp/kala-frozen-conductor-build.log` (build complete in ~131 s).
The exact fix the conductor verified is the packaging-side
`module_collection_mode={"kala_agent": "pyz+py"}` plus
`collect_submodules("kala_agent")` so the core's startup
`module.__file__.stat()` digest works under `_MEIPASS`. Delivery made
**no** PyInstaller build of its own; the binary is a pre-final
snapshot and the conductor will rebuild after all writers stop.

The smoke validator launches that binary from a fresh temporary cwd
(so the repo-relative `studio/dist` cannot be discovered) and observed:

```
[smoke] health ok: {"integrations": [...11...], "offline": false,
  "providers": ["local","openai","openrouter","codex"],
  "service": "kala-studio", "status": "ok",
  "step_available": true, "version": "0.1.0"}
[smoke] UI root and assets ok.
[smoke] preview mesh ok.
[smoke] STEP export: 15518 bytes, ISO-10303-21 magic ok.
[smoke] PASS
```

Smoke robustness choices: no exact STEP byte count is asserted (OCP
version and header timestamps change the size); it requires the
`ISO-10303-21` magic and `END-ISO-10303-21` terminator, `200`,
`application/step`, and `attachment` disposition. The exact geometry
round-trip is covered by the studio pytest suite. Child stdout is
capped at 200 lines, the child is reaped with bounded
terminate-then-kill in `finally`, the port is a free loopback port,
and the token is synthetic with provider keys cleared. STEP is
mandatory whenever health advertises `step_available`; CI never passes
`--allow-missing-step`.

## CI matrix

`.github/workflows/studio.yml` has three real jobs:

1. `build` — matrix `[ubuntu-latest, windows-latest]`:
   install `.[dev,studio,geometry]` (fail-closed) + `pyinstaller`,
   `npm ci`, `npm run build`, `npm run typecheck`, `npm test`,
   studio lint/type gates (`ruff check` + `ruff format --check` over
   `src/kala_agent/studio` and the studio test files, and
   `mypy --strict src/kala_agent/studio`), studio pytest via globs
   (`tests/security/test_studio*.py tests/unit/test_studio*.py`, all
   9 files), PyInstaller build, frozen-binary smoke on both OSes,
   copy binary into `studio/backend`, digest,
   `npm run desktop:pack -- --publish never`, upload unsigned
   artefacts with `if-no-files-found: error`.
2. `core-tests-linux` — full root suite with the geometry extra.
3. `adapter-tests-linux` — FreeCAD adapter suite, `-m "not freecad"`.

Node is pinned to `22` (studio requires `>=22.12` for Vite 8 /
Vitest 5). There is no self-referential "confirm no secret references"
step, no `studio-smoke-windows` pseudo-job, and no `continue-on-error`.
The workflow uses `permissions: contents: read` and references no
secrets.

## Remaining blockers (not delivery-owned)

1. **Windows runtime is unverified.** The Windows build path is
   configured, but no Windows runtime has been exercised for this
   wave; no clean Windows host run was performed.
2. **electron-builder desktop bundle not yet validated.** The UI cell
   owns `studio/electron-builder.json` (schema repair in progress).
   Delivery only verifies that the backend binary exists before
   `desktop:pack` runs and that the artefact upload fails closed if
   nothing is produced. The final Linux AppImage build is still
   pending the parent build, and no Windows package has been built
   or run.
3. **CI has not run on real runners.** This host has no GitHub
   Actions runners, so the matrix is validated structurally
   (`yaml.safe_load`, `yamllint`, line-length and banned-token scans)
   and every Linux step has a local equivalent that was run. The
   Windows steps have no local equivalent here.
4. **End-to-end install on a clean host is not performed**, consistent
   with the wave's honesty disclaimer.

## What this delivery did not do

- No PyInstaller rebuild (conductor owns the final source build).
- No core, backend, UI, or `src/kala_agent/studio/**` edits.
- No delegation, commits, pushes, deploys, or credential reads.
- No live provider or solver call, and no paid fallback.
- No signed installer and no tested clean-host install.
