# Cell ortho-sheet (Phase B): the full-sheet 2D orthographic drawing with clickable vectors and the Feature Edit Popover

Read `../../WAVE.md` in full: D1, D5, D11, the contract and the rules. Read `../../SPEC.md` § 4C.1 and 4C.3, and § 5.3. Read `../sheet-shell/REPORT.md` for the frozen stub props, `chatTurn.ts` and `scriptBuild.ts`. Read `../edit-turns/REPORT.md` for the edit-turn contract.

## Do
1. **`OrthoSheet.tsx`** (shown by AppShell when `viewMode === 'ortho2d'`; it fills the drafting field).
   - **With an active design:**
     - The `/drawing` views (via the shared design controller) in third-angle layout: TOP above FRONT, RIGHT to the right of FRONT. Use ONE common mm→px scale for all three, fitted to the field.
     - Show view labels (e.g. `TOP · LOOKING −Z`) and a small third-angle projection symbol.
     - Hidden lines dashed in `--ink-muted`.
     - Overall dimension lines from `design.measured.bounds` (real values, mm, two decimals).
     - Loading, error and truncated states are explicit.
     - Publish `{source:'ortho2d', pxPerMm, gridMm}` through `components/drafting/scaleBus.ts` (it exists; do not change it).
   - **With only a primitive:** today's silhouettes from `computeOrthoViews`, with hover length badges. No popover.
2. **Vectors.** Turn every polyline into features:
   - A closed polyline whose points fit a circle (max radial deviation ≤ max(0.5% r, the drawing's chord tolerance)) becomes ONE `circle`. An open circular run becomes an `arc`.
   - Otherwise, consecutive collinear segments (within 0.5°) merge into one `segment`, so a tessellated edge reads as one line.
   - Each feature is an SVG element with an invisible 8 px hit stroke.
   - **Hover:** stroke `--accent-redline`; a live badge near the pointer (`L = 60.00 mm`, `Ø 12.00 mm`, `R 6.00 mm`); `feature/hover` with a full `FeatureRef` (bbox in view mm). Clear on leave.
   - **Keyboard:** the sheet is one focusable widget. ←/→ cycle features (with the same hover effects), Enter opens the popover, Esc closes. It has an aria label naming the keys.
   - Put the feature extraction in a pure module (`components/ortho/features.ts`) and unit-test it hard.
3. **Feature Edit Popover** (on click or Enter; `feature/select`). Anchored to the vector, with focus management; Esc closes.
   - **Header:** view, kind, measure, and "hidden line" when hidden.
   - **Parametric nudge (D5).**
     - Find the script steps whose SIZE parameters equal the feature's dimension within 0.01 mm. Size parameters are widths, heights, lengths and depths, radii (match Ø = 2r and R = r) and diameters. Never match positions or angles.
     - Read the real op and field names from the construction contract (`backend/crates/kala-contracts/src/construction.rs` and its docs). Parse `design.script` defensively.
     - Each match shows `Component · step i · op · field` with a stepper (−1, −0.1, value, +0.1, +1) and Apply.
     - Apply deep-clones the script, sets the value, and calls `buildScript({script, provider: selected CAD target ?? design.provider, label: '<label> (edited)', activate:true})`. Show the building state and the backend's refusal text verbatim.
     - **Delete feature:** only for a matched `pocket` step; removes that step, then rebuilds.
     - **No match:** say "No script step has this dimension."
   - **Scoped micro-prompt.**
     - A single-line input ("Change this edge/hole…"). Submit calls `sendTurn({prompt, baseDesignId: design.design_id, focus})`.
     - `focus` = `${VIEW} view · ${kind} · ${measure} · from (u1, v1) to (u2, v2) mm${hidden ? ' · hidden line' : ''}`, capped at 1024 chars.
     - Show the `turnReadiness` blocker when not ready.
   - **Extension hook (SPEC § 5.3):** the popover's sections come from an internal array of section renderers, so a future add-on can register one. Register only the two real sections.
4. **Tests:** feature extraction (a circle, an arc, a merged collinear edge, a hidden edge), FeatureRef bbox and measures, keyboard cycling, the popover's nudge matching (diameter vs radius, no position match), the patched script, delete-for-pocket-only, the focus string, and the micro-prompt call (mock `chatTurn`/`scriptBuild`).

## Own
- `studio/src/components/ortho/**`.
- `studio/src/viewer/HlrDrawing.tsx`, `viewer/hlr.ts`, `viewer/draftingGeometry.ts`. Other cells import these, so keep the existing exports working.
- The new `studio/src/styles/ortho.css`, and the tests of these.
- The store, types and AppShell are frozen: use "Requests" in the REPORT.

## Gates
`npm run typecheck`, `npm test`, `npm run build` in `studio/`. Other cells edit concurrently: if a failure is in a file you do not own, re-run once, then report it. No commits. Clean up /tmp. Write `REPORT.md`.
