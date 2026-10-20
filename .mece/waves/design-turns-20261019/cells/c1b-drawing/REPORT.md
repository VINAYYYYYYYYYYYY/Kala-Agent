# C1b-drawing — 2D HLR sheet, design tabs, title block, AuxView

Cell: `c1b-drawing`, wave `design-turns-20261019`.
Spec: `../c1b-components/TASK.md` (including the binding Opus decisions and the
"backend without design support" addendum), ADR-0009 §7, the C1a store
contract, the B1 API report, and the C0b design-system report.

Status: **complete for the owned surface**. Typecheck, my five suites, and the
production build are green. The full `npm test` currently has **one failure in
a parallel worker's file** (`test/nativeBackendPackaging.test.ts`, the Rust
wave's readiness count) that appeared after `src/kala_agent/studio/internal_api.py`
landed at 23:39:59; details in §9. No screenshots, commit, push, delegation,
credential read, Python edit, or `studio/electron/**` / `studio/scripts/**` /
`electron-builder.json` edit was performed.

## 1. Files

### Created

| File | Lines | Purpose |
|---|---:|---|
| `studio/src/viewer/hlr.ts` | 251 | Pure HLR drawing geometry: polyline extents, aspect-preserving cell fit with a flipped SVG axis, deterministic path strings, pan/zoom transform maths, and the measured-envelope/view-axis helpers. |
| `studio/src/viewer/HlrDrawing.tsx` | 433 | The 2D Pencil static drawing: iso view large, front/top/right small in third-angle, 2B visible / dashed 2H hidden polylines, measured redline callouts, wheel zoom, drag pan, reset, and loading/error/truncation states. |
| `studio/src/components/drafting/useDesignController.ts` | 33 | One shared `DesignController` for the sheet and the AuxView, created and disposed with the ready connection (StrictMode-safe). |
| `studio/test/hlr.test.ts` | 156 | 13 pure tests for fit/paths/zoom/envelope axes. |
| `studio/test/HlrDrawing.dom.test.tsx` | 200 | 12 DOM tests: four views, line classes, measured callouts, pan/zoom/reset, loading/error/retry. |
| `studio/test/useDesignController.dom.test.tsx` | 98 | 2 DOM tests: no controller before `connection/ready`, then a real request into the drawing cache. |

### Edited (owned)

| File | Change |
|---|---|
| `studio/src/components/drafting/DraftingSheet.tsx` | Design sheet tabs, design title block, HLR/3D stage switch (canvas unmounted for the drawing), design mesh in clay, design cut-note bounds, measure disabled on the static sheet, `+` composer starter. |
| `studio/src/components/drafting/AuxView.tsx` | Design Section (loops/evenodd/hatch/exact area/plane selector), design Ortho (HLR front/top/right), design Tolerance (declared list), loading/error/retry states, no-design-backend note. Primitive panels are unchanged. |
| `studio/src/styles/drafting.css` | Design tab strip, HLR drawing/controls/callouts, Aux design panels, plane selector, loading/error/declared styles. Tokens only; no raw px `font-size`. |
| `studio/src/viewer/SceneCanvas.tsx` | `meshSource` accepts `'design'`; optional `designMesh` flag skips the primitive construction-wireframe pass and labels the source "OCP measured design mesh". |
| `studio/test/DraftingSheet.dom.test.tsx` | +9 tests: HLR/canvas exclusivity, clay design mesh, design title block, design cut note, tab strip/activation/close/`+`, no-backend strip. |
| `studio/test/AuxView.dom.test.tsx` | +12 tests: design section (exact/approx/plane/loading/error), design ortho, declared tolerance list, no-backend note (false and unreported), primitive fallbacks. |

### Minimal wiring outside the slice

| File | Change |
|---|---|
| `studio/src/components/shell/AppShell.tsx` | **AppShell was re-read immediately before patching.** Added one import (`useDesignController`), one hook call (`const designController = useDesignController()`), and one prop on each of `DraftingSheet` and `AuxView`. No other logic changed. |
| `studio/src/state/store.ts` | **Not edited.** The closed-tab set is local UI state; the store's design LRU remains the session record. |

## 2. Contract added

```ts
// viewer/hlr.ts
interface DrawingCell { x; y; width; height }
interface PolylineExtent { minU; minV; maxU; maxV; width; height }
interface PolylineFit { extent; scale; originX; originY; paths: string[] }
interface DrawingTransform { zoom; panX; panY }
polylinesExtent(groups): PolylineExtent | null
fitPolylineGroups(groups, cell, padding): PolylineFit | null
polylinePath(polyline, map): string
clampDrawingZoom(zoom): number
zoomDrawingAt(transform, point, factor): DrawingTransform
panDrawing(transform, dx, dy): DrawingTransform
drawingTransformAttribute(transform): string
measuredEnvelope(measured): { x; y; z } | null
measuredAxisValue(envelope, axis): number
VIEW_MEASURED_AXES // front X/Z, top X/Y, right Y/Z — A2's published view basis

// viewer/HlrDrawing.tsx
interface HlrDrawingProps {
  design: DesignResult;
  drawing: DrawingResponse | null;
  status: 'idle' | 'loading' | 'ready' | 'error';
  error: string | null;
  showDimensions: boolean;
  viewResetToken: number;
  onRetry?(): void;
}

// components/drafting/DraftingSheet.tsx
interface DraftingSheetProps {
  /* existing props unchanged */
  designController?: DesignController | null;
  primitive?: PrimitiveKind;       // shapes the clay design mesh only
  parameters?: StudioParameters;
}
DESIGN_STARTER_PROMPT: string
prefillComposer(prompt: string): void

// components/drafting/AuxView.tsx
interface AuxViewProps {
  /* existing props unchanged */
  designController?: DesignController | null;
}

// viewer/SceneCanvas.tsx
meshSource: 'local' | 'server' | 'design' | 'none';
designMesh?: boolean;
```

## 3. The 2D Pencil vector drawing

- **Exactly one WebGL surface.** `DraftingSheet` renders `HlrDrawing` and does
  **not** render `SceneCanvas` when `renderMode === 'pencil2d'` and a design is
  active. The DOM test asserts `.viewer-canvas` is absent and `.hlr-drawing`
  is present. Clay mode keeps the interactive `SceneCanvas` with the design's
  own measured OCP mesh (`designToDisplayMesh`), memoised so unrelated shell
  renders do not rebuild the GPU geometry. Pencil mode without a design keeps
  the existing three.js pencil rendering.
- **Layout.** Iso view large on the left; front, top, and right on the right
  in third angle (top above front, right of front; the empty top-right cell
  states the projection convention). Each view is fitted independently with
  aspect preserved; the visible and hidden groups of one view share a single
  transform.
- **Linework.** Visible polylines use `.hlr-visible-line` (ink, 2B weight);
  hidden polylines use `.hlr-hidden-line` (construction ink, `5 3` dash).
  `vector-effect: non-scaling-stroke` keeps the drafting weight while zooming.
- **Measured redline callouts.** The six dimension lines read
  `design.measured.bounds` (not `declared_envelope`): front X/Z, top X/Y,
  right Y/Z, plus the full `MEASURED ENVELOPE · X … · Y … · Z …` note. They
  disappear with the existing Dimensions toggle.
- **Pan / zoom / reset.** Wheel zoom is cursor-centred (0.5×–8×); pointer drag
  pans; the on-sheet `− / % / + / RESET` controls and the draft palette's
  Reset View (`viewResetToken`) reset the transform. A design change also
  resets it.
- **Honest states.** Loading names the request; a failure shows the real
  controller error with a retry; a view the backend omitted reads
  `ISO VIEW NOT RETURNED`; `truncated` is reported per view and in the
  heading. The sheet legend says `NO SCALE CLAIM`.
- **Section hatching** lives in AuxView's Section panel, where the cut faces
  exist; the static HLR sheet has no cut face to hatch.

## 4. Sheet tabs = designs

- When `state.server.designAvailable === true`, the tab strip is built from
  `state.designs.order` (the LRU). Each tab shows the design label and the
  first 8 characters of its id. Selecting a tab dispatches `activateDesign`,
  so the active tab is bound to `activeDesignId`.
- A close button removes the tab from the strip only. The design remains in
  the store LRU (tested: `order.length` and `designs.has(id)` are unchanged);
  activating a closed design from anywhere reopens its tab. Closing the
  active tab activates the nearest remaining tab, or clears the active design.
- The `+` tab calls `prefillComposer(DESIGN_STARTER_PROMPT)`: it focuses the
  real `textarea.chat-panel-prompt` and writes the starter through the native
  value setter (React's value tracker stays in sync). It dispatches no design
  action and creates no sheet.
- On `designAvailable !== true` the design strip is absent and the original
  static `SHEET 01` label remains, so primitive/preview work is unchanged.

## 5. Title block

With an active design the title block renders only backend data:

| Cell | Value |
|---|---|
| SUBJECT // DESIGN ID | `design_id.slice(0, 8)` (full id in `title`) |
| DRAWING | `label` |
| MATERIAL (DECLARED) | `material`, or `—` |
| TOLERANCES (DECLARED) | the declared strings joined with ` · `, or `—` |
| SCALE | `FIT` (the views are fitted independently) |
| UNITS | `METRIC (MM)` |
| PROJECTION | `3RD ANGLE` |
| REV | `—` |

There is no checker, author, revision number, or date cell in design mode:
`DesignResult` carries no journal timestamp, so no date is invented. Without
a design, the original primitive title block is unchanged.

## 6. AuxView

- **Section (design).** A plane selector (`XZ` default, `XY`, `YZ`) requests
  `getDesignSection(designId, plane)` through the shared controller. The
  loops render as one `fill-rule="evenodd"` path with the ANSI 45° hatch
  pattern and an ink boundary. The footer and the SVG show the measured
  `area_mm2` (`≈` and `POLYLINE AREA` when `area_exact` is false), the returned
  `offset_mm`, the outer/inner loop counts, and a truncation note.
- **Ortho (design).** `getDesignDrawing(designId)`'s front/top/right views in
  third-angle arrangement, with visible/dashed styling, real view names, an
  `OCP HLR` stamp, and a truncation note. Missing views read `NOT RETURNED`.
- **Tolerance (design).** The declared strings as a list under
  `declared — not analysed`; material is labelled `(declared)`; the panel
  states that no fits, GD&T, or tolerance stacks are computed.
- **Loading/error.** Section and Ortho each have a named loading state, the
  controller's real error message, and a retry that re-requests the same
  resource. The primitive panels are unchanged.
- **No-design backend.** With `design_available: false` the Tolerance tab
  shows exactly: “Design turns need the Python geometry backend; this backend
  reports design_available: false”. With the flag absent (`null`) it says
  “this backend did not report design_available” rather than claiming a value
  the backend never sent. Primitive Section, Ortho, and the existing tolerance
  empty state keep working.

## 7. Tests

| File | Tests | Covers |
|---|---:|---|
| `test/hlr.test.ts` | 13 | extents, aspect fit, axis flip, closed `Z`, straight-line fit, degenerate input, cursor-fixed zoom, zoom clamp, pan, transform string, measured envelope and view axes |
| `test/HlrDrawing.dom.test.tsx` | 12 | iso + three small views, visible/hidden classes, missing view, truncation, six measured callouts, dimensions off, wheel/control zoom, reset button/token, pointer pan, loading, error/retry |
| `test/DraftingSheet.dom.test.tsx` | 18 (9 new) | HLR replaces the canvas, clay keeps the design mesh, pencil without a design, design title block, measured cut note, tab strip/activation/close/`+`, no-backend strip, plus the original frame/palette/section tests |
| `test/AuxView.dom.test.tsx` | 23 (12 new) | design section exact/approx/plane/loading/error, design ortho + loading/error, declared tolerances, no-backend false/unreported and primitive fallbacks, plus the original primitive panel tests |
| `test/useDesignController.dom.test.tsx` | 2 | no controller before a ready connection; a ready connection requests the drawing and writes the correct cache key |

Total in my five files: **68 passed**. The two pre-existing suites
(`DraftingSheet`, `AuxView`) were extended, not rewritten; no test was deleted.

## 8. Gates (in `studio/`)

| Command | Result |
|---|---|
| `npm run typecheck` | **clean**, exit 0 |
| `npm test` | **39 files / 531 tests passed, 1 failed**: `test/nativeBackendPackaging.test.ts` “reports the actual 14 Python route-methods versus the native 9” (received 18). This is the parallel Rust wave's test; see §9. |
| `npm run build` | **green**, 66 modules transformed, built in 659 ms; only the pre-existing >500 kB `three` chunk warning |
| `npx vitest run` (this cell's five files) | **68 passed** |

The same full suite was **532/532 green at 23:39:44**, before
`src/kala_agent/studio/internal_api.py` (an ADR-0010 worker file, not this
cell) was written at 23:39:59. The readiness scan then reported 18 Python
route-methods against the Rust test's stale pinned expectation of 14.
Re-running the failing file twice produced the same result. I did not edit the
Python route file, the readiness script, or the Rust wave's test. The baseline
before this cell was 33 files / 441 tests.

## 9. Gaps and honest notes

**Parallel-worker gate note.** The final `npm test` reports 531/532 because
`test/nativeBackendPackaging.test.ts` (the Rust backend wave) pins “14 Python
route-methods” and the readiness scan now counts 18. The count changed when
`src/kala_agent/studio/internal_api.py` was created at 23:39:59 by a parallel
worker; the full suite was 532/532 green at 23:39:44. The failing assertion and
the readiness script are outside my owned files, and the wave instructions
forbid touching `studio/scripts/**` and the Python backend, so I left them
alone and re-ran twice to confirm the failure is stable.

1. **Plane offset is the backend default.** The Section plane selector sends
   no `offset`, so the backend cuts at its documented default (the Y mid-plane
   for `XZ`). The returned `offset_mm` is displayed; there is no offset slider
   (not in the cell scope).
2. **The static sheet ignores the Grid/Axes toggles.** Those controls remain
   the 3D viewport's controls; the HLR sheet has its own view frames and the
   vellum grid. The measure tool is explicitly disabled on the static sheet
   (the tooltip says so).
3. **The 3D cut plane and the Aux section are different cuts.** The existing
   three.js redline plane is the Z mid-plane of the measured bounds; the Aux
   Section is the backend `/section` cut on the selected plane. The cut note
   names the 3D plane it actually draws. Neither is presented as the other.
4. **No design timestamp exists in `DesignResult`.** The title block therefore
   has no date cell (the task allows one only if B1 provides it); no date or
   checker is invented.
5. **Tolerances are declared metadata only.** The list is shown verbatim with
   `declared — not analysed`; no fit, stack, or GD&T result is computed.
6. **Closed tabs are session-local UI state.** They are not persisted and do
   not evict or delete the design. A design re-activated by the agent card (or
   a new turn) reappears in the strip.
7. **The `+` starter needs a mounted composer.** If the chat panel is not in
   the DOM (BootGate or a bare sheet test), the helper focuses nothing and
   creates nothing. It also does not auto-send; it only fills and focuses the
   field.
8. **HLR loading follows the mode.** The sheet requests all four views on
   entering 2D Pencil; Aux Ortho requests the same cached key when opened from
   Clay. The C1a cache means switching modes does not re-download.
9. **No live backend was used in this cell's tests.** The drawing/section
   payloads are the B1 recorded fixtures plus the store caches; the controller
   hook test stubs the HTTP client. Screenshot QA against a live local-helper
   turn is left to D1, as instructed.
