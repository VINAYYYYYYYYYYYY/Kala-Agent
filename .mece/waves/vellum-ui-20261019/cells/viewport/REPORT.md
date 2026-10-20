# P2a viewport — report

Status: **complete**. Vellum drafting sheet, pencil/clay viewport, real
redline dimensions, Section A–A, orthographic three-view, and the honest
tolerance empty state are implemented in `studio/`. No commit, push, deploy,
credential read, or delegation was performed. No files outside `studio/**`
were touched except this report.

## Gate results

Run from `/home/v9i/Projects/Kala-Agent/studio`:

| Command | Result |
| --- | --- |
| `npm run typecheck` | clean (exit 0) |
| `npm test` | **18 test files passed, 220 tests passed** (baseline 14 / 178; P2b added files concurrently) |
| `npm run build` | success, 50 modules transformed; only the pre-existing >500 kB `three` chunk warning |

The P2a suites specifically add **30 tests**: 13 geometry-math tests, 11
AuxView tests, 6 DraftingSheet tests. The full run was repeated after the
last CSS/lighting change and stayed green.

## Files changed

New:

- `studio/src/viewer/draftingGeometry.ts`
- `studio/src/styles/drafting.css`
- `studio/test/draftingGeometry.test.ts`
- `studio/test/AuxView.dom.test.tsx`
- `studio/test/DraftingSheet.dom.test.tsx`

Modified:

- `studio/src/viewer/SceneCanvas.tsx` (full renderer rewrite)
- `studio/src/components/drafting/DraftingSheet.tsx`
- `studio/src/components/drafting/AuxView.tsx`
- `studio/src/components/shell/AppShell.tsx` — **one line**: title-block
  `projection: 'PERSPECTIVE'` → `'3RD ANGLE'` as required by the cell brief.
  Re-read immediately before patching.
- `studio/test/AgentWorkspace.dom.test.tsx` — **one assertion block** in the
  existing AuxView tab test: the old placeholder sentence
  (`No projected three-view drawing`) was replaced with assertions that three
  real `.aux-ortho-view` groups render and `THIRD-ANGLE` is stated. No
  behavioural coverage was removed.
- `.mece/waves/vellum-ui-20261019/cells/viewport/REPORT.md` (this file)

Not touched: `studio/src/state/store.ts`, `studio/src/types/studio.ts`,
`studio/src/main.tsx`, `studio/index.html`, `studio/electron/**`,
`studio/scripts/**`, `studio/package.json`, `electron-builder.json`, backend
Python/Rust/solidedge, chat/journal files owned by P2b.

## Contracts

The frozen P1 contracts are unchanged:

- `DraftingSheetProps` and `TitleBlockData` — unchanged.
- `AuxViewProps` — unchanged.
- No store fields or actions were added or changed.

Internal extension to the P2-owned `SceneCanvas`:

```ts
export interface SceneCanvasProps {
  // ...existing P1 props...
  renderMode: RenderMode;
  showSection: boolean;
  viewResetToken: number;
}
```

`DraftingSheet` no longer remounts the canvas with `key={viewResetToken}`.
The token is forwarded to `SceneCanvas`, which re-frames the live camera
(`resetView()`) without recreating the WebGL renderer.

New pure geometry API in `src/viewer/draftingGeometry.ts`:

```ts
computeSectionView(kind, parameters): SectionView
computeOrthoViews(kind, parameters, bounds): OrthoView[] | null
fitToBox(outerW, outerH, contentW, contentH, padding): FittedBox
isCircularExtent(a, b): boolean
formatMmValue(value): string
formatMmDimension(value): string
```

## What was implemented

### DraftingSheet

- Double drafted frame (1.5 px + 0.5 px), A–F left/right and 1–8 top/bottom
  coordinate labels with boundary ticks and four corner registration crosses.
- Pencil grid composed from the existing `--grid-pattern` token plus a
  drafting `+` tick pattern at major intersections; paper grain is an inline
  SVG `feTurbulence` data URI. No network image and no CSP change.
- Floating `✎ 2D Pencil Vector | ⬡ 3D Clay` pill bound to `renderMode`.
- Four-button vertical palette, every one wired to a real action: Reset view,
  Toggle dimensions, Toggle Section A-A, Toggle grid. No dead buttons.
- Redline A–A edge flags and a real `CUT A–A · <plane>` note when the cut is
  on. The 3D cutting plane itself is drawn in the renderer.
- Title block uses only the supplied data: `STUDIO-<KIND>`, drawing title from
  the primitive dimensions, `FIT` scale, `METRIC (MM)`, `3RD ANGLE`, sheet,
  and `—` fallbacks for an empty tolerance/revision/author.

### SceneCanvas

- `pencil2d`: cream/vellum Lambert faces (roughness-free, near-flat), 2B ink
  `EdgesGeometry`, dashed 2H hidden-edge pass using `GreaterDepth` +
  `LineDashedMaterial`, and a 24° FOV camera for an orthographic feel.
- `clay3d`: warm clay `MeshStandardMaterial` with `roughness: 1`,
  `metalness: 0`, soft hemisphere light, and the same graphite edge overlay.
- Scene background is transparent for `paper`/`dark`, so the vellum sheet
  (grid + grain) shows through. `studio` keeps the old opaque technical
  backdrop.
- Grid and custom axes use pencil colours; both are rebuilt on theme change.
  A `MutationObserver` watches `data-theme` and re-themes the face, edges,
  dimensions, section plane, grid, and axes live. Slate clay and slate pencil
  were both exercised.
- Real redline dimension callouts: Three.js dimension lines plus HTML labels
  projected from the bounds corners; labels re-project while orbiting.
- Redline Section A–A cutting-plane rectangle at the real `z = 0` mid-plane.
- Generic disposal walks geometries and materials on every mesh, mode, theme,
  and theme/material swap; `ResizeObserver` and the theme observer are
  disconnected on unmount. No animation loop exists, so
  `prefers-reduced-motion` is inherently respected.

### AuxView

- Section A–A: rectangle for box (`size_x × size_y`), circle for cylinder at
  the mid-plane normal to its axis, circle for sphere at the great circle.
  Unsupported primitives get an explicit “Section A–A unavailable” state.
  Profiles are filled with an SVG 45° ANSI hatch and dimensioned with redline
  arrows showing real millimetres.
- Ortho 3-View: front/top/right silhouettes from the live mesh AABB, laid out
  in third-angle order (top above front, right of front). Cylinder front view
  is a circle only when its real X/Y extents are circular; otherwise it falls
  back to an honest rectangle. No bounds means an honest “run a preview”
  empty state.
- Tolerance: explicit empty state. It says the backend exposes no tolerance
  stack/GD&T/fit class and shows `—` / `Not provided`; no limits are invented.
- Tabs are keyboard navigable (←/→/Home/End), with `aria-selected`, roving
  `tabIndex`, `aria-controls`, and `aria-labelledby`.

## Browser verification (real data)

Launched the real `.venv` backend on `127.0.0.1:8765` serving the built dist,
connected through the normal browser-fallback form, and drove headless
Chromium over CDP at **1440×900** and **1280×800** in both themes.

| Check | Result |
| --- | --- |
| Columns at 1440 | `340 / 780 / 320`, shell 900 high, sheet 852, AuxView exactly 250 |
| Columns at 1280 | `280 / 740 / 260`, sheet 752, AuxView 250 |
| Horizontal/vertical overflow | `0 / 0` at both sizes |
| `SceneCanvas` count | exactly 1 through mode switches, theme switches, drawer opens, primitive changes |
| WebGL | real canvas rendered (not the fallback) |
| Pencil → clay | face, edge, hidden-line, lighting, and camera FOV switch live |
| Vellum → slate | grid, axes, face, edges, dims, section, and Aux hatch re-theme live |
| Palette toggles | dims host `display:none` / restored; A flags and cut note removed / restored; section attr on/off |
| Primitive changes | box rectangle → cylinder circle (⌀40.00, normal to axis) → sphere circle (⌀50.00, great circle); one canvas throughout |
| Title block | real `STUDIO-BOX`, `40.00 × 20.00 × 30.00 mm`, `FIT`, `METRIC (MM)`, `3RD ANGLE`, `TOLERANCE —` |
| Console | no app exceptions; see the font note below |

## Known gaps / honest notes

1. **Camera is narrow-FOV perspective, not a true orthographic camera.** The
   24° pencil FOV reads as orthographic and keeps OrbitControls; a real
   OrthographicCamera switch was not implemented.
2. **Section A–A is the centred `z = 0` mid-plane.** That is exact for all
   three centred primitives, but there is no operator-movable cutting plane or
   arbitrary section in the backend, so no other offset is offered.
3. **Revision/author are shell-provided strings** (`DRAFT`, `Kala Studio`),
   not backend revision metadata. The sheet renders `—` if either is empty.
   Scale remains `FIT`; `1:1` is deliberately not claimed.
4. **Tolerance is entirely unavailable** in this build and is presented as
   such. No fit class, GD&T, or limit is computed.
5. **Orthographic silhouettes are AABB projections.** Curved primitives use
   circles only when the real projected extents are circular; otherwise a
   rectangle is drawn rather than inventing a curve.
6. **Font CSP mismatch observed (outside P2a ownership).** The backend response
   header in `src/kala_agent/studio/middleware.py` is `font-src 'self'`, while
   `studio/index.html` allows `font-src 'self' data:`. The built CSS contains a
   small @fontsource woff2 as a data URI, which Chromium blocks in
   browser-fallback mode (console error; system fallback fonts render). No
   CSP, backend, or font packaging change was made by P2a. This is a
   pre-existing integration issue for P3/conductor.
7. **Pre-existing build warning:** the `three` chunk remains >500 kB. No
   splitting change was made.

No fake placeholder data is used in any P2a component. Every number shown is
derived from the active parameters or live mesh bounds, or is an explicit
`—` / “not provided” state.
