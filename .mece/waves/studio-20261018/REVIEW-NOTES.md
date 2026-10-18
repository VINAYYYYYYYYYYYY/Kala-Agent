# Conductor review notes (in progress)

These observations are for follow-up worker repairs after the first reports; they are not verification claims.

## UI

- `BootGate.tsx` currently sets `phase: connecting` after successful registry loading and never enters ready; verify/fix before finalizing.
- Browser bootstrap uses `window.prompt`, which blocks collaborative-browser automation and is poor onboarding. Prefer an accessible in-page form defaulting to `window.location.origin`, token in memory only, avoid StrictMode duplicate prompts. The backend should not print secrets by default; don't claim it prints a token unless deliberately implemented/documented for the manual flow.
- Coordinate dimensions/tessellation field names, bounds format, error shape, status enums, and capabilities shape with actual backend modules once written; WAVE only specified top-level shape.
- Preview controller timeout abort currently returns without clearing loading state. Check stale responses on disposal, abort previous request immediately on parameter edits, abort registries on unmount, bound registry fetches, avoid uncaught abort errors.
- Inspect WebGL resource disposal on material changes/grid/axes; prefer render-on-demand (including orbit updates) over continuous idle animation to reduce laptop CPU/GPU use.
- Remove dead placeholders such as `currentBoundsOrNull()` always returning null and unused lastFingerprint variables if not used.
- Dependencies initially chosen are years behind installed registry versions. On October 18, 2026 npm registry returned Electron 44.9.0, Vite 8.3.4, electron-builder 26.15.3, Three 0.186.1, React 19.3.1. At minimum use a supported secure Electron and run `npm audit`; select compatible dependency set, don't blindly upgrade with force. Node installed version to be checked.
- Respect requested code style: don't add large blocks of explanatory inline comments or one-letter variable names; use concise identifiers and necessary docs in README instead.

## Packaging

- Verify frozen static path matches backend's `_MEIPASS` resolution, and output location/build CLI scripts match spec, electron-builder and CI.
- Actual `/home/v9i/.local/bin/codex` is a mise wrapper; `codex --version` stalled for over 90 seconds and was terminated. Do not claim authenticated Codex CLI was tested; make unavailable/auth/startup timeouts actionable.
- Initial spec references `__file__` even though PyInstaller spec execution supplies `SPECPATH`/`SPEC` globals, not necessarily `__file__`; test actual PyInstaller build. Bundle destination `studio_dist` must match frozen server lookup exactly. Missing UI must fail release packaging, not merely warn.
- CI must fail typecheck and missing desktop artifact checks; remove `continue-on-error` for typecheck and `if-no-files-found: ignore` for desktop output. Avoid claiming success in always-run summary when earlier steps failed. Use a Node version compatible with updated Vite (installed local Node is 26.8.2; current Vite requires ^20.19 or >=22.12).
- Initial provider docs speculated unsupported behavior (active geometry in chat request, CLI error returned verbatim, missing key refusing server startup). Align every behavioral claim with actual backend, sanitize CLI errors, avoid unsupported legal/terms assertions.
- Actual PyInstaller smoke invocation failed `studio/dist not found` even though repo `studio/dist/index.html` exists: `SPECPATH` is already a DIRECTORY, so `Path(SPECPATH).resolve().parent` incorrectly removes `packaging`. Use directory directly; only `.parent` on SPEC full file fallback. Re-run real build, not just syntax parsing.

## Backend geometry (independent check)

At the first stable-geometry review, independent triangle checks found:

- `build_box(20,30,40)`: 6 of 12 triangles wound inward; signed volume 0 instead of 24,000 mm^3.
- `build_cylinder(10,30)`: 96 of 192 triangles wound inward; signed volume -3,132.63 rather than approximately 9,424.78 mm^3.
- `build_sphere(10)`: 32 of 1,472 triangles degenerate; normal unit lengths are fine but nondegenerate closed triangulation is required.

The check takes each triangle's cross product `(second-first) x (third-first)`, requires dot with its centroid > 0 for these origin-centered convex primitives, and sums `dot(first,cross)/6` for signed volume. Test every face/cap and all pole triangles, not just selected normals/indices. Check welded-by-position manifold edges have exactly two uses and opposite orientations. Add regression tests and fix generators before release.

## Backend capability / cross-platform concerns

- `geometry/step.py` initially probes OCP but explicitly does NOT implement writer; that does not meet brief. OCP is now installed in project .venv as cadquery-ocp 8.0.1.1.0. Implement primitive B-rep using `BRepPrimAPI_MakeBox/MakeCylinder/MakeSphere` and `STEPControl_Writer`; export underlying primitive parameters, NOT the tessellated shell, preserve centered coordinate frames. Round-trip STEP with OCP reader, validate BRepCheck/volume/bounds. If runtime absent return 503; availability means writer usable, not import alone. Make real STEP handoff usable.
- Windows `os.fchmod` is not available; token-file handling must use a platform-safe secure approach and avoid startup AttributeError. Do not label `O_TRUNC` in-place writing atomic; use actual same-directory atomic replace with restricted file and directory. Caller-supplied env token needn't create a disk copy.
- Studio parent package imports composition root automatically via kala_agent/__init__.py; avoiding studio middleware core import does not bypass Python parent package initialization. Don't break core public API or edit unowned core files; packaging should reflect actual dependency graph.
- Official Codex CLI invocation is an opt-in tool-capable local subprocess. Read-only sandbox still permits reads and configured plugins may have external authority; isolated cwd does not prove full host isolation. Don't promise otherwise. Consider an additional explicit enable flag, sanitize startup/auth errors, never return raw stderr or prompt containing credentials. Prevent inherited API key environment from silently turning the subscription route into separately billed API mode. No live paid/auth call for tests.

## Follow-up observations during repair (to verify before final)

- Conductor re-ran actual updated geometry: box/cylinder now have 0 inward/degenerate triangles, valid welded-by-position edges. Sphere still has **67 invalid welded edges** and mesh volume 4,212.03 mm^3 > exact 4,188.79 for r10. Root cause visible: body loop `for ring_index in range(1, ring_count)` includes final latitude ring -> `ring_first[ring_count]` is pole index0, then `+sector_index` indexes unrelated vertices. Body must only connect intermediate rings (`range(1, ring_count-1)`), then separate pole fans. Regression test welded positions / paired reversed directions, not index topology or only triangle normal test.
- Exact Studio helper `export_step(kind=...,parameters=...)` now successfully roundtrips through OCP for all 3 shapes and correct volumes. Exporter imports OCP.IFSelect/OCP.STEPControl BEFORE `_require_ocp`, however, so missing dependency can raise ModuleNotFoundError instead of documented StepUnavailableError; reorder availability guard. Validate canonical defaults/ranges consistently with mesh; bounds and STEP must agree even when API `parameters:{}` uses defaults.
- Registry initial revision advertises IGES although no exporter/schema supports it. Capabilities must only include actually supported formats. AutoCAD claim '2018+ can import STEP' is overbroad; native support depends product edition/toolsets. Simcenter Nastran/OptiStruct STEP goes through a CAD preprocessor, not directly solver. Avoid claiming existing FreeCAD adapter already consumes Studio requests without wiring evidence.
- UI audit after initial dependency refresh found **2 critical +9 moderate dev-dependency advisories**. `vitest^3` is vulnerable (and tinypool transitively), npm registry latest vitest is5.0.5 with Node22.12/24/26 support. Upgrade compatible test stack; run both full and production-only audit after final lockfile. No forcing incompatible global overrides. electron-builder26 stable may have unresolved moderate advisories: report clearly if no safe stable fix.
- Codex provider draft uses invented `codex exec --no-shell`, omits sandbox and git-safety flags, and dispatch codex branch does not enforce `allow_remote`. Official docs fetched October18: https://developers.openai.com/codex/noninteractive documents `codex exec --sandbox read-only --skip-git-repo-check --ephemeral` and prompt-on-stdin `codex exec -`; `--ignore-user-config` exists for controlled automation. Use documented flags, explicit read-only, opt-in enable and consent. `CODEX_API_KEY` must be filtered too (currently preserved through CODEX_ prefix); don't expose prompt via process arguments if stdin supported. Current raw stderr/stdout error excerpt must never reach API/logs. subprocess.run(capture_output=True) is NOT bounded memory; use capped concurrent reading and terminate on byte limit, bound deadline, reap child. Mock these paths; no live billing/auth tests.
- Actual dev server starts and API same-origin requests now200. BUT `/` and assets404: static.resolve_static_root iterates `source(environ)` for functions `_candidate_from_meipass()` and `_candidate_from_repo()` that accept NO arguments, then swallows TypeError. Repo/frozen static roots never work! Use correctly called functions; test source/frozen/env routes and missing-asset404. `REPO_DIST_SUBDIR` also annotated str but assigned tuple. Main browser QA currently uses explicit `--static-dist` only as a workaround; normal desktop MUST work without this workaround.
- `App` repair creates controller when connection changes, but second trigger effect only depends primitive/parameters; initial connection becoming ready doesn't trigger any preview. Add current connection/controller generation to trigger dependency or trigger initial mesh from creation effect. `BootGate` aliveRef cleanup sets .alive=false and next StrictMode setup never sets true/new handle; this still breaks development bootstrap. Add real React component lifecycle tests, not only helper functions.

## Existing baseline

- Core lint passed.
- FreeCAD lint passed when run from adapters/freecad using its own config. Running with --config from root misclassifies test imports; use intended cwd.
- Four pre-existing FreeCAD files fail formatting: mod/Kala/kala_gui.py, src/kala_agent_freecad/cad/accept.py, tests/test_compose.py, tests/test_executor_live.py. Pure formatting may be assigned as a bounded cleanup after main implementation, with no behavioral changes.
