# Cell canvas-3d (Phase B) — report

The full-bleed 3D drafting field: orthographic camera, zoom-synchronised
millimetre grid, graphite / CAD-colour / wire finishes, the vertical tool
strip, the view switcher and the honest scale pill.

## Gates

| Gate | Result |
|---|---|
| `npm run typecheck` in `studio/` | **pass** (exit 0) |
| `npm test` in `studio/` | **1185 passed / 1185**, 86 files |
| `npm run build` in `studio/` | **pass** (exit 0; only the pre-existing three.js chunk-size warning) |

Each gate ran with `RUSTUP_HOME=/home/v9i/.rustup`, `CARGO_HOME=/home/v9i/.cargo`, and a fresh
`mktemp -d` for `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR`, `XDG_DATA_HOME` and
`npm_config_cache`. No `/home/v9i/.local/share/kala-agent` path was touched, no credential/auth/settings
file was read, and no real `claude`/`codex`/`opencode` CLI was run. Temp dirs were removed.

## Files changed

### Created

- `studio/src/viewer/projection.ts` — pure orthographic maths: px-per-mm from `camera.zoom`, the
  major-grid selection (10/50/100 mm kept near the 40–200 px band), minor spacing and smoothstep fade,
  the `1:N` ratio at the 0.2646 mm CSS reference, drafting-ratio formatting (never a false `1:1`),
  the standard scale ladder and its zoom stepping, and `orthoFitZoom`.
- `studio/src/viewer/cadColor.ts` — vertex welding by quantised position, union-find triangle islands,
  the eight-colour engineering palette, per-vertex colours and the honest `cadColorPlan` rule
  (`design.view.solids` mismatch → one colour + "solids not separable in the view").
- `studio/src/viewer/snap.ts` — nearest projected vertex within 8 CSS px (pure; the viewport projects
  the mesh on the click only).
- `studio/src/viewer/graphite.ts` — the graphite shader (warm paper highlight, lambert-driven
  three-layer screen-space cross-hatch, three `gl_FrontFacing` handling), the constant-width 1.2 px ink
  silhouette (one screen-space back-face pass), and the 30° crease-edge fat lines
  (`three/examples/jsm/lines`).
- `studio/src/viewer/gridLayer.ts` — the camera-facing shader plane: 0.6 px major lines at
  `rgba(24,22,20,0.11)`, 1 px minor dots at `rgba(24,22,20,0.22)` times the fade, coloured from the
  theme token `--canvas-grid-ink`.
- `studio/src/styles/canvas.css` — the field palette tokens (vellum + blueprint slate), the full-bleed
  canvas override (the decorative sheet dots are gone), the tool strip, view/finish switcher, scale pill
  and CAD legend, all with `:focus-visible` rings.
- Tests: `test/projection.test.ts`, `test/cadColor.test.ts`, `test/snap.test.ts`,
  `test/graphite.test.ts`, `test/ToolStrip.dom.test.tsx`, `test/ViewSwitcher.dom.test.tsx`,
  `test/ScalePill.dom.test.tsx`.

### Modified (owned)

- `studio/src/viewer/SceneCanvas.tsx` — rewritten around an `OrthographicCamera` whose frustum is the
  container in CSS pixels, so `camera.zoom` **is** the px-per-mm scale. The shader grid, the three
  finishes, the CAD legend, vertex snapping, scale-lock wheel/keyboard stepping, the paused hidden
  state, and `publishScale({source:'model3d', pxPerMm, gridMm})` on every camera/size/grid change.
  OrbitControls orbit/pan/reset/dolly and the orthographic raycast all work unchanged.
- `studio/src/components/drafting/DraftingSheet.tsx` — now only the full-bleed canvas: one
  `SceneCanvas` mounted for the session, hidden and paused in `ortho2d`, keeping design/primitive mesh
  selection, the 3D section redline, caliper picking, Escape-clear and the real server-preview verdict.
  The old frame, zone ticks, title block, HLR paragraph, envelope text, three-view inset, 2D/3D pill,
  design tab strip and tool palette are removed.
- `studio/src/components/drafting/ToolStrip.tsx` — the real 32×32 vertical strip: reset isometric,
  dimensions, caliper (+ live readout), Section A-A, grid, axes, snap-to-vertex + scale lock (both
  store flags), and the disabled `aria-disabled` add-on slot with the exact tooltip.
- `studio/src/components/drafting/ViewSwitcher.tsx` — `[3D Model] [2D Orthographic]` plus the 3D-only
  `◐ Graphite Shade` / `● CAD Color` / `Wire` sub-toggle.
- `studio/src/components/drafting/ScalePill.tsx` — `GRID = 10 MM · SCALE ≈ 1:N` from `useSheetScale()`,
  standard drafting labels, em dash for unknowns, and the monitor-dependency tooltip.
- `studio/src/styles/drafting.css` — moved the viewport source label below the top-right switcher.
- Tests rewritten/updated where they asserted the old layout: `test/DraftingSheet.dom.test.tsx`
  (new structure + hidden round trip + no #2/#7/#8/#14 text), `test/MeasureTool.dom.test.tsx`
  (button now in the ToolStrip), `test/SceneCanvas.dom.test.tsx` (orthographic label, hidden state,
  CAD legend).

### Shared tests touched (minimal, old-layout assertions only)

- `test/shellStubs.dom.test.tsx` — removed the three Phase A stub assertions for ToolStrip, ViewSwitcher
  and ScalePill; the other cells' stub tests are untouched.
- `test/AgentWorkspace.dom.test.tsx` — the Phase B mount-point test now asserts the real `.tool-strip`,
  `.view-switcher` and `.scale-pill` inside their WAVE overlay slots (the four remaining stubs are
  unchanged).

`studio/src/state/store.ts`, `studio/src/types/studio.ts`, `AppShell.tsx`,
`components/drafting/scaleBus.ts` and `useDesignController.ts` were **not** modified.

## Contract consumed

- Types: `ViewMode`, `ModelFinish`, `PreviewResponse`, `StudioLocalPreview`, `CadProviderId`, `BBox`.
- Store read: `viewMode`, `finish`, `viewport.{showGrid,showAxes,showDimensions,snapToGrid,scaleLock}`,
  `showSection`, `sectionCut`, `measure`, `preview`, `viewResetToken`, `designs` (active design + view
  solids).
- Store dispatched: `view/reset`, `viewport/set`, `section/toggle`, `measure/toggle`, `measure/point`,
  `measure/clear`, `viewMode/set`, `finish/set`.
- `components/drafting/scaleBus.ts`: `publishScale` / `useSheetScale` — interface untouched.
- `DraftingSheet` props and `ToolStrip` / `ViewSwitcher` / `ScalePill` no-prop contracts exactly as
  frozen by sheet-shell.

No contract fields or actions were added.

## Behaviour notes

- **Grid.** A camera-facing shader plane sits behind the part on the target's view ray. An orthographic
  projection makes its depth irrelevant to spacing, so the squares are real model millimetres and the
  plane can never paint over the part. Major lines are 0.6 px; dots fade between 3 px and 8 px apart.
  The 3D source publishes `pxPerMm` and `gridMm` (null while the grid is off or the view is hidden).
- **Scale.** `N = 1 / (pxPerMm × 0.2646)`. Standard labels are used within 2 %, `1:1` only at exactly
  N = 1; rounding can never print `1:1`. Scale lock steps the wheel and the keyboard zoom through
  1:50 … 5:1 at the reference pixel, and snaps the current zoom when it is switched on.
- **Graphite.** One `EdgesGeometry` (30°) as 1.2 px fat lines plus one 1.2 px back-face silhouette pass,
  both built once per mesh. The face shader hatches in screen space (three staggered layers).
- **CAD colour.** Weld, union-find, palette — computed once per mesh in a memo, never per frame. The
  chip prints `CAD COLOR · per solid · N solids`, or "solids not separable in the view" on a mismatch.
- **Hidden canvas.** `hidden` + `display:none`; `renderNow` is a no-op, resizes are ignored, the pill is
  reset to unknown, and the last real size/camera is kept so the round trip does not steal the zoom.

## Tests added (behavioural)

- Grid: option set, readable-band choice, both fallbacks, minor spacing/fade.
- Scale: ratio maths and round trip, exact-only `1:1`, standard labels, honest 3-figure ratios,
  ladder labels, next-step stepping both ways and at the ends, orthographic fit.
- Islands: two disjoint boxes → 2; one box with per-face duplicated vertices (24 view vertices) → 1;
  empty mesh; palette wrap; the legend fallback.
- Snapping: nearest within 8 px, exact-radius boundary, miss, empty/nonsense input.
- Graphite: three hatch layers from the lambert term, screen-space hatching, 30° crease, 1.2 px edges,
  back-face silhouette.
- Every tool-strip action including `aria-pressed` (and `mixed`), the disabled add-on and its tooltip,
  the caliper enable/readout, and the focus-ring CSS.
- The view switcher and the 3D-only finish sub-toggle.
- The scale pill: unknown, exact `1:1`, an honest 1:3.78, an ortho2d publication, the tooltip.
- The field structure: one canvas through the ortho round trip, no #2/#7/#8/#14 markup or text, design
  mesh + solid count, shared section cut, caliper wiring, Escape, preview refusal/in-flight.

## Known gaps

1. **Pinch zoom and scale lock.** The wheel and the keyboard step between standard scales; a trackpad
   pinch (OrbitControls' two-pointer dolly) still free-zooms. Fixing it needs a controls-level dolly hook.
2. **Vertex snapping cost.** The snap projects every mesh vertex on each pick (two picks per measurement),
   never per frame. On a million-triangle view that is a one-off pass per click, not a running cost.
3. **CAD welding memory.** Welding keys are position strings, so the first CAD-colour switch on a very
   large design is a one-time O(n) allocation. Island count also merges solids that touch at exactly one
   welded vertex; the honest fallback then reports "solids not separable in the view".
4. **Transitional props.** `DraftingSheet` still accepts `material`, `renderMode`, `titleBlock` and
   `units` because `AppShell` is frozen; nothing in the field reads them anymore.
5. **`snapToGrid` name.** The store flag keeps its old name; the tool now means "snap the caliper to the
   nearest mesh vertex within 8 px", as the brief specifies.
6. **Legend scope.** The CAD chip only appears while CAD colour is active and a mesh exists; a primitive
   has no `design.view.solids`, so its honest island count is printed as the solid count.
7. **Retired components.** `HlrDrawing`, `AuxView`, `BottomBar` and `OptionsDrawer` are unreferenced by
   the live shell but still in the tree (qa removes them with their coverage; I did not touch files I do
   not own).

## Requests

None blocking. For qa / Opus, when the legacy components are deleted:

1. Drop `material` / `renderMode` from `DraftingSheetProps` and from `AppShell`'s call, and retire
   `RenderMode` / `MaterialKind` (the live state is `viewMode` + `finish`).
2. If scale lock should also govern pinch zoom, add a small controls-level dolly hook; no contract change
   is needed for it.
