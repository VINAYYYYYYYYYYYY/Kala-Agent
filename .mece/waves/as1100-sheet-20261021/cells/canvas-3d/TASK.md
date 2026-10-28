# Cell canvas-3d (Phase B): the 3D model view, the grid, the tool strip, the view switcher and the scale pill

Read `../../WAVE.md` in full: decisions D1, D8, D9, D13, D17, the contract and the rules. Read `../../SPEC.md` § 4A, 4B.2 and 4C.1–2. Read `../sheet-shell/REPORT.md` for the stub props, which are frozen.

## Do
1. **`DraftingSheet.tsx` becomes the full-bleed 3D canvas of the drafting field.**
   - Remove its own frame, coordinate ticks, title block, the floating HLR paragraph (#2), the measured-envelope/ISO text (#8), the three-view inset (#7) and the 2D/3D pill (#14). The shell frame, the dock and the ortho sheet replace them.
   - Exactly one `SceneCanvas` stays mounted. When `viewMode === 'ortho2d'`, the canvas stays mounted but hidden, with its render loop paused, so the camera survives a round trip.
   - Keep the design/primitive mesh selection, the section-cut redline in 3D, measure picking, and viewport keyboard control (arrows orbit, +/− zoom, 0 resets, focus ring, aria label).
2. **Orthographic camera (D9).**
   - `SceneCanvas` switches from `PerspectiveCamera(24)` to an `OrthographicCamera`, with a default isometric framing that fits the mesh. Zoom changes `camera.zoom`.
   - Orbit, pan, reset and raycast measure must work with it. Update the viewer tests.
3. **Zoom-synchronised grid (SPEC § 4A).**
   - A screen-space background grid, aligned to the view plane through the target, whose spacing is real model millimetres at the current zoom.
   - Major lines: 0.6 px, `rgba(24,22,20,0.11)` (theme tokens in slate). Pick 10, 50 or 100 mm, whichever keeps the on-screen spacing in a readable band (about 40–200 px).
   - Minor dots: 1 px, `rgba(24,22,20,0.22)`, at 1 or 5 mm, fading in smoothly as their spacing grows.
   - Toggled by the grid tool. Publish `{source:'model3d', pxPerMm, gridMm}` through `components/drafting/scaleBus.ts` (it exists: do not change its interface).
4. **`ScalePill.tsx`** (bottom-left of the drafting field).
   - Reads `useSheetScale()` and shows `GRID = 10 MM · SCALE ≈ 1:N`, per D9.
   - N comes from px-per-mm at the CSS reference pixel (0.2646 mm). Use standard drafting ratios formatting (`2:1`, `1:2.5`).
   - The tooltip explains that the screen ratio depends on the monitor. Unknown values → "—". Never "1:1" unless exact at the reference.
5. **`ToolStrip.tsx`.** Vertical, 32×32 monochrome buttons, `aria-pressed` on toggles, visible focus ring.
   - **Reset isometric view:** `view/reset`.
   - **Dimensions and leaders:** `viewport.showDimensions`.
   - **Point-to-point caliper:** the existing measure tool; the readout stays real.
   - **Section A-A:** `section/toggle`.
   - **Grid** and **Axes** toggles: these move here from the bottom bar.
   - **Snap and scale lock:** sets both `snapToGrid` and `scaleLock`. Scale lock makes zoom step between standard scales: 5:1, 2:1, 1:1, 1:2, 1:5, 1:10, 1:20 and 1:50 at the reference pixel. Snap makes caliper points snap to the nearest mesh vertex within 8 px; the label says "snap to vertex".
   - **`+` add-on slot (SPEC § 5.1):** rendered disabled (`aria-disabled`), with the tooltip "Add-on slot — no add-ons installed".
6. **`ViewSwitcher.tsx`.**
   - A segmented `[3D Model] [2D Orthographic]` control (`viewMode/set`).
   - In 3D, a finish sub-toggle: `◐ Graphite Shade`, `● CAD Color`, `Wire` (`finish/set`).
7. **Graphite shade.** A custom shader for the default finish, theme-aware (slate: light ink on blueprint):
   - warm paper highlights;
   - two or three layers of screen-space cross-hatching in the shadow troughs, driven by the lambert term;
   - crisp ink silhouette and crease edges (crease angle about 30°; `three/examples/jsm` fat lines are allowed for the 1.2 px width).
   - It must stay interactive on a 1M-triangle mesh: build edges once per mesh, never per frame.
8. **CAD Color (D8).**
   - Weld vertices by quantised position (the view duplicates vertices per face), union-find triangles into islands, and colour each island from an 8-colour engineering palette. Compute this once per mesh, off the render path.
   - The legend chip reads `CAD COLOR · per solid · N solids`.
   - If the island count differs from `design.view.solids`, fall back to one colour, and the chip says "solids not separable in the view".
9. **Wire:** today's wireframe material.
10. **Tests:** grid spacing selection, scale ratio formatting, island colouring (two disjoint boxes → 2; one box with per-face duplicated vertices → 1), scale-lock zoom steps, every tool-strip action, the view switcher, and the DraftingSheet structure (no #2/#7/#8 text).

## Own
- `studio/src/viewer/**`, EXCEPT `HlrDrawing.tsx`, `hlr.ts` and `draftingGeometry.ts` (ortho-sheet owns those; you may import them).
- `studio/src/components/drafting/DraftingSheet.tsx`, `ToolStrip.tsx`, `ViewSwitcher.tsx`, `ScalePill.tsx` and `useDesignController.ts` (keep its interface).
- `studio/src/styles/drafting.css`, a new `styles/canvas.css`, and the tests of these.
- The store, types and AppShell are frozen: write needed changes under "Requests" in the REPORT.

## Gates
`npm run typecheck`, `npm test`, `npm run build` in `studio/`. Other cells edit concurrently: if a failure is in a file you do not own, re-run once, then report it. No commits. Clean up /tmp. Write `REPORT.md`.
