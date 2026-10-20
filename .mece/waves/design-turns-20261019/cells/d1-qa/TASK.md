# D1 qa: end-to-end verification, kept short

Read `../../WAVE.md`, ADR-0009, and the REPORTs of c1a-data, c1b-drawing, c1b-ledger, and c1b-agent. You may fix integration and cosmetic breakage in `studio/src/**` and `studio/test/**` only: no new features, no Python, nothing under electron/scripts.
1. Gates: `npm run typecheck && npm test && npm run build`; `.venv/bin/python -m pytest tests/unit -q`; ruff and mypy on `src/kala_agent/studio`.
2. Launch the Python Studio backend serving `studio/dist` (method in `/home/v9i/Projects/Kala-Agent/.mece/waves/vellum-ui-20261019/cells/qa/REPORT.md`) and drive headless Chromium at 1440x900. Use the local helper with the L-bracket sentence that the B1 parser accepts, and verify on screen:
   - volume 35571.42 mm³ (or the exact formatting) in the agent card and the bar;
   - B-Rep Verified;
   - the 2D pencil SVG drawing with hidden dashed lines;
   - 3D clay;
   - the Section tab area 2321.46 mm²;
   - the ledger with HASH-CHAIN OK after verify;
   - a STEP export receipt with sha256;
   - the "make me a bracket" turn asks for values and creates no design.
   Then simulate a backend with `design_available: false` (a route stub or env, as the C1b REPORTs describe) and confirm the honest degraded mode.
3. Save about 8 screenshots to `shots/` (vellum 2D, clay, section, ledger, export receipt, slate, 1280, degraded mode).
4. Write `REPORT.md`: gate counts, a checklist with pass/fail per item above, and any remaining gaps. Keep the whole run under ~45 minutes; do not polish beyond fixing breakage.

## Fix list from C1b reports (Opus-triaged; fix these first)
1. `studio/src/App.tsx`: the journal poller starts when health reports `design_available: false` (for example the Python backend without OCP), then polls routes that 503. Start it ONLY when `design_available === true`. Add a test.
2. The journal ledger forces horizontal scrolling in the 320px rail below 1400px. Add a compact mode: under ~1400px show SEQ | TYPE | MESSAGE | DIGEST and move UTC and CHAIN into the row `title` / an expandable detail. No horizontal scrollbar at 1280 or 1440. Add a layout test.
3. With an active design the Options drawer export card grows. Re-measure: all four cards fully visible at 1440x900 with a design active (as C0b guaranteed without one). Fix by bounding the receipt area if needed.

## Additional authorised fix (Opus is now also the Rust-wave conductor)
`studio/test/nativeBackendPackaging.test.ts` "reports the actual 14 Python route-methods" fails (it sees 18), because `studio/scripts/native-readiness.cjs` counts every `@router.*` under `src/kala_agent/studio/`, including the worker-only `internal_api.py` router (`APIRouter(prefix=INTERNAL_PREFIX)`, mounted only in `--worker` mode, never public). You MAY edit `studio/scripts/native-readiness.cjs` and that test for this ONLY: count only public `/api/studio` route-methods, report internal worker route-methods separately as `internalRouteMethodCount` (and list them), and keep the "unknown public additions fail the diff" guard intact. Add a test proving an internal route is not counted as public, and that a new public route still fails the diff.

## Context note
ADR-0011 (new) makes external CAD providers the default builders and OpenCascade view-only unless opted in. The current UI flow you are testing equals provider=opencascade (opt-in). Test it as is; provider routing and the selector come in later cells (B2/C2). Do not implement them.
