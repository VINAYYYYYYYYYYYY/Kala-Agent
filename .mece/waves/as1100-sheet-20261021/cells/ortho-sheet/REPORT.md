# Cell ortho-sheet (Phase B) — report

Status: **complete**. The full-sheet 2D orthographic drawing, its clickable
features and the Feature Edit Popover are implemented against the frozen
`OrthoSheet({ designController })` props. No commits. No files outside the
cell's ownership were changed.

## Gates

| Gate | Result |
|---|---|
| `npm run typecheck` in `studio/` | pass (exit 0) |
| `npm test` in `studio/` | **1193 passed / 1193** in **86 files** |
| `npm run build` in `studio/` | pass (exit 0; only the pre-existing three.js chunk-size warning) |

My five test files contribute **89 tests** (baseline when this cell started was
817; the suite's growth beyond 906 is the other Phase B cells' work).
Every gate ran with `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR`,
`XDG_DATA_HOME` under a fresh `mktemp -d` and `npm_config_cache=/home/v9i/.npm`.
No `/home/v9i/.local/share/kala-agent` path was touched, no credential/auth/
settings file was read, no real `claude`/`codex`/`opencode` CLI was run. Temp
dirs were removed.

## Files changed

### Created (all owned)

- `studio/src/components/ortho/OrthoSheet.tsx` — the sheet: third-angle layout,
  design HLR vectors, primitive silhouettes, keyboard widget, scale publishing,
  explicit loading / error / truncated / empty states, and the top-layer popover
  mount (a `createPortal` at z 180, above the graphite string and the drafting
  overlays, below the engineer card's z 200).
- `studio/src/components/ortho/features.ts` — the pure feature extractor:
  `circle` / `arc` / merged `segment`, `FeatureRef` builder and identity, the
  `L` / `Ø` / `R` measures, and the scoped-edit `focus` string.
- `studio/src/components/ortho/nudge.ts` — the pure parametric-nudge engine:
  size-field table from the construction contract, 0.01 mm matching with the
  `Ø = 2r` / `R = r` relations, defensive script parsing, deep-cloned patch and
  pocket-step deletion.
- `studio/src/components/ortho/layout.ts` — the pure third-angle page layout:
  one shared mm→px scale, cell placement, view-plane→page mapping, arc/segment
  SVG paths, and the "nice" major-grid spacing.
- `studio/src/components/ortho/FeaturePopover.tsx` — the editor: header, the
  parametric-nudge section, the scoped micro-prompt, the section registry
  (SPEC § 5.3), focus management and Escape.
- `studio/src/styles/ortho.css` — the sheet and popover styling (tokens only;
  no raw px font sizes, no rounded corners, no animation).
- Tests: `test/orthoFeatures.test.ts` (22), `test/orthoNudge.test.ts` (19),
  `test/orthoLayout.test.ts` (10), `test/OrthoSheet.dom.test.tsx` (19),
  `test/FeaturePopover.dom.test.tsx` (19).

### Owned but deliberately untouched

- `studio/src/viewer/HlrDrawing.tsx`, `studio/src/viewer/hlr.ts`,
  `studio/src/viewer/draftingGeometry.ts` — every existing export is still
  imported by `DraftingSheet`, `AuxView`, `SceneCanvas` and `BottomBar`, so they
  were left byte-identical. The new sheet reuses `polylinesExtent`,
  `designViewCaption`, `VIEW_MEASURED_AXES`, `measuredAxisValue`,
  `measuredEnvelope`, `formatMmValue` and `computeOrthoViews` from them rather
  than forking a second rule.

### Not touched

`studio/src/types/studio.ts`, `studio/src/state/store.ts`,
`studio/src/components/shell/AppShell.tsx`, `studio/src/state/chatTurn.ts`,
`studio/src/state/scriptBuild.ts`, `studio/src/components/drafting/scaleBus.ts`,
`studio/electron/**`, `studio/scripts/**`, `electron-builder.json`,
`index.html` and everything under `backend/`.

## Contract consumed (unchanged)

- **Props**: `OrthoSheet({ designController: DesignController | null })`.
- **Types**: `FeatureRef` (exactly the frozen field set), `OrthoViewName`,
  `DrawingResponse` / `DrawingView` / `DrawingPolyline`, `DesignResult` and its
  `script` / `measured.bounds`, `CHAT_FOCUS_MAX_CHARS` (1024).
- **Store**: `selectActiveDesign`, `selectDrawing`, `selectDesigns` (ownership
  only), `useUiState`, `dispatch`; actions `feature/hover`, `feature/select`;
  selectors `selectHoveredFeature`, `selectSelectedFeature`,
  `selectCadProvider`, `selectPreview`, `selectPrimitive`, `selectParameters`.
- **Scale bus**: `publishScale({ source: 'ortho2d', pxPerMm, gridMm })` and
  `getScale()`; nothing in `scaleBus.ts` changed. `pxPerMm` is published only
  from a measured field; before measurement it is `null` (the pill prints "—",
  never a guessed ratio). The published `gridMm` is the real model spacing of
  the major grid lines the sheet draws (chosen so their on-screen pitch is at
  least 32 CSS px).
- **Turn machine**: `sendTurn({ prompt, baseDesignId, focus })` and
  `turnReadiness(state)` from `state/chatTurn.ts`. The micro-prompt treats
  **every** blocker as hard (WAVE Phase A decision) and shows the readiness
  message; `sendTurn`'s own `{sent, blocker, message}` refusal is shown when it
  still declines.
- **No-model rebuild**: `buildScript({ script, provider, label, activate })`
  from `state/scriptBuild.ts`, with `provider = selected CAD target ??
  design.provider`, `label = "<design.label> (edited)"`, `activate: true`.
  `ScriptBuildError.message` is rendered verbatim.
- **Backend edit-turn contract**: `base_design_id` is the active
  `design.design_id`; `focus` is ≤ 1024 characters and carries no control
  characters. The `edit-turns` REPORT is present and implements exactly the
  WAVE contract (`focus_requires_base_design` is an extra defensive code this
  cell can never trigger, because it always sends both).
- **Construction contract**: the nudge's size fields are read from
  `ComponentStep` in `backend/crates/kala-contracts/src/construction.rs`:
  `width`, `height`, `length`, `pitch`, `diameter` (plus `depth` and `radius`
  read defensively). `x`, `y`, `angle`, `plane`, `profile`, `op` and
  `reversed` are never matched. `pocket.length` is the cut depth
  (`_cut_depth`), which is why a matched pocket is the only deletable step.

## Behaviour as implemented

### The sheet

- **Design mode.** Requests the shared `designId|all` drawing key through the
  one controller (no duplicate request in StrictMode; a failure is left to the
  Retry button). TOP above FRONT, RIGHT to the right of FRONT, one common scale
  fitted to the measured field, the empty quadrant above RIGHT carrying the
  third-angle symbol. View labels are `designViewCaption` (`TOP · LOOKING −Z`,
  `FRONT · LOOKING +Y`, `RIGHT · LOOKING −X`), so a caption can never name a
  different direction from the polylines. Hidden lines are dashed
  `--ink-muted`; visible lines are the primary ink; the overall dimensions are
  the provider's `measured.bounds` values on each view's published axes, two
  decimals, drawn as extension-line-and-tick dimensions. Loading, error (with
  retry), truncated (per view and page-level), no-controller, backend-refused
  (`design_available: false`) and empty-drawing states are all explicit.
- **Primitive mode.** `computeOrthoViews` silhouettes from the live mesh
  bounds, with the label, the real width × height / Ø text and a hover badge
  (`L = … mm · H = … mm`). No popover.
- **Vectors.** Every polyline becomes features. A closed run that fits a circle
  to `max(0.5% r, chord_tolerance_mm)` **at its vertices and chord midpoints**
  is one `circle`; an open run is an `arc`; everything else merges consecutive
  collinear segments (0.5°, including across a closed polyline's seam). Each
  feature is an SVG group with a visible path and an invisible 8 px
  (`vector-effect: non-scaling-stroke`) hit stroke. Hover inks the vector
  `--accent-redline`, shows the live badge next to the pointer (`L = 60.00 mm`,
  `Ø 12.00 mm`, `R 6.00 mm`) and dispatches `feature/hover` with the full
  `FeatureRef` (view-plane mm bbox); leaving clears it.
- **Keyboard.** The sheet is one focusable widget (`role="group"`, `tabIndex 0`)
  whose aria label names ←/→/Enter/Escape. With the editor open the arrows move
  the editor with the highlight; keys typed in its fields or pressed on its
  buttons are left alone. Enter opens; Escape closes and clears the hover.
- **Popover.** Anchored on the selected vector (`FeatureRef.bbox` mapped through
  the view placement), fixed-position top layer, focus taken on open and
  returned to the sheet on close, Escape closes, outside press closes, presses
  on another vector retarget it.

### The Feature Edit Popover (D5)

- Header: uppercase view · kind, the measure, and `hidden line` when hidden.
- **Parametric nudge.** Every `components[].steps[]` size parameter equal to the
  feature's dimension within `0.01 mm`, each shown as
  `Component · step i · op · field` (step numbers are 1-based, matching the
  operator's reading order) with `−1`, `−0.1`, an editable value, `+0.1`, `+1`
  and Apply. Values are bounded to the contract's `0.001–5000 mm`. Apply
  deep-clones the script, sets the field, calls `buildScript` with the CAD
  target (or the design's provider), shows `Building…` + `Rebuilding through
  <provider> — no model call.` with a **Cancel build** button, and renders
  `ScriptBuildError.message` verbatim. A cancellation (code `cancelled`) reads
  `Build cancelled.` as a status, not an error. **Delete feature** is offered
  only when the matched step's `op` is `pocket`; it removes the pocket **and**
  the sketch it consumes, and its tooltip says exactly what leaves
  (`removes sketch + pocket, steps 3..4`). When the preceding step is not a
  sketch, only the pocket is removed and the tooltip says `removes step 2`.
  With no match: `No script step has this dimension.`
- **Scoped micro-prompt.** One single-line input (`Change this edge/hole…`),
  submit → `sendTurn({ prompt, baseDesignId: design.design_id, focus })`. The
  focus string is built from the extracted geometry's **real** points:
  `TOP view · segment · L = 60.00 mm · from (10.00, 0.00) to (0.00, 10.00) mm`
  for an edge, and
  `FRONT view · circle · Ø 10.00 mm · centre (0.00, 24.00), Ø 10.00 mm`
  (or `… centre (u, v), R r`) for a bore/arc, plus ` · hidden line` when hidden
  and capped at 1024 characters. The readiness blocker is shown before submit
  and a refused send is reported instead of pretending it left.
- **Extension slot.** The sections come from a module-level array of
  `{ id, title, Component }`; `registerFeaturePopoverSection` adds one (returns
  its unregister) and only the two real sections are registered. A future weld
  symbol / surface-finish mark is an add-on, not a change to this cell.

## Known gaps

1. **Nudge parsing reads the canonical flat step fields.** `design.script` is
   the backend's canonical script (flat `width`/`diameter`/…), and that is what
   is parsed. A hand-written `profile: { rectangle: { width } }` spelling is not
   scanned; the backend flattens it before returning a design anyway.
2. **`pitch` is matched as a length-type size** because `ComponentStep` defines
   it as a distance (`helix` pitch). `depth` and `radius` are supported
   defensively even though the current construction step does not accept them.
3. **`data-stub="ortho-sheet"` and the `No drawing loaded.` empty text are
   kept** on the real component so the unowned `shellStubs.dom.test.tsx` and
   `AgentWorkspace.dom.test.tsx` mount-point assertions keep passing. qa can
   drop the attribute when it rewrites those assertions.
4. **Cancelling a nudge/delete build is a client-side cancellation.** The
   AbortController closes the client request and the editor reports
   `Build cancelled.`, but the backend's `POST /api/studio/design` may still
   finish the build it already started; its result is never shown (the request
   is gone) and the design store is not touched. The same rule the chat turn
   uses for a timeout applies: the UI says what it knows, not what it hopes.
5. **Primitive mode has no zoom/pan and no per-vector selection by design ("No
   popover").**
6. **The popover and the badge are portaled to `document.body` and positioned
   `fixed`.** The application document does not scroll, so viewport coordinates
   are the sheet's coordinates; embedding the sheet in a transformed/scrolling
   container would need the portal's origin passed in instead.

## Follow-up 1 (Opus review, 2026-10-21)

All four follow-ups landed inside the cell's own files; no frozen file changed.

1. **A pocket deletes as a unit.** `planStepDeletion` in `nudge.ts` now decides
   what a delete removes: the matched `pocket` plus the sketch immediately
   before it when that step really is a `sketch`, otherwise the pocket alone.
   `deleteStep` applies that plan to a deep copy. The Delete button's native
   tooltip is the plan's description — `removes sketch + pocket, steps 3..4`
   or `removes step 2` — so the operator sees exactly what leaves before
   clicking. Tests cover both cases (`orthoNudge`: the sketch+pocket plan and
   the no-sketch plan; `FeaturePopover`: both tooltips and both rebuilt step
   lists), and the old test that asserted the sketch was left behind is gone.
2. **The focus string uses the real geometry.** `focusForFeature(feature,
   geometry?)` now takes the extracted `OrthoFeature`: a segment names its
   first and last sampled points (the merged run's true endpoints), and a
   circle/arc names its centre plus `Ø d` / `R r`. Without geometry it falls
   back to the frozen ref's bbox corners, so an externally restored `FeatureRef`
   still produces a usable string. `OrthoSheet` finds the open feature's
   geometry by `featureKey` and hands it to the popover as a new (cell-owned)
   `geometry` prop. The pure test uses a diagonal edge whose bbox corners are
   deliberately *not* its endpoints: `from (10.00, 0.00) to (0.00, 10.00) mm`.
   Circle and arc centre forms are tested too.
3. **A rebuild can be cancelled.** The nudge section owns one `AbortController`
   per build, passes `signal` to `buildScript`, shows a **Cancel build** button
   beside the building status, and aborts on unmount. A rejection with
   `ScriptBuildError.code === 'cancelled'` renders `Build cancelled.` as a
   `role="status"` line (never an `alert`, never a red refusal) and leaves the
   editor open; every other failure keeps the verbatim refusal. Tests assert
   the signal is passed, that Cancel aborts it and shows the non-error line, and
   that unmounting the editor aborts a build still in flight.
4. **The hover badge is in the top layer.** The badge is now a `createPortal`
   child of `document.body` with `position: fixed`, `z-index: 170` (above the
   tool strip/view switcher/pencil at 12–14 and the graphite string at 40, below
   the feature editor at 180 and the engineer card at 200) and a
   `data-layer="top"` marker. Its anchor is the pointer (or the feature box for
   the keyboard cycle) in viewport pixels. The DOM tests assert the badge is not
   inside the sheet container, carries `data-layer="top"`, and still shows the
   right `Ø` / `L`+`H` text in both design and primitive modes.

Gates after the follow-ups (same isolated HOME/XDG setup; temp dirs removed):
`npm run typecheck` pass; `npm test` **1193/1193 in 86 files** (89 of them
this cell's); `npm run build` pass (only the pre-existing three.js chunk-size
warning). No file outside the cell's ownership was modified; the title-block
cell was editing concurrently and its files were not touched.


## Requests

None. The frozen store, types, `AppShell`, `chatTurn` and `scriptBuild` were
sufficient; no contract change is needed for this cell.
