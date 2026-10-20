# P2a viewport: vellum drafting sheet, pencil/clay rendering, AuxView

Read `../../WAVE.md` first, then `../shell/REPORT.md`, which defines the binding prop interfaces and store contract. Work only in `/home/v9i/Projects/Kala-Agent/studio`. You own `src/viewer/**`, `src/components/drafting/**`, `src/styles/drafting.css`, and the matching tests. The P2b worker is editing chat/journal files at the same time, so don't touch those. AppShell/store edits are limited to minimal additive wiring: re-read the file right before patching and list the edit in your REPORT.

Reference: `stitch/kala_studio_the_vellum_drafting_board_master_workspace/` (center sheet and right-top aux card), plus `drafting_canvas_workbench/` and `drawing_sheets_title_blocks/`.

## DraftingSheet (center; touches the top window edge)
- Vellum sheet: double drafted border, zone ticks A–F (left/right) and 1–8 (top/bottom), subtle pencil grid (major/minor token colours, small + ticks at major intersections), and optional paper grain made with CSS/SVG only (no images from the network).
- Title block at bottom-left, drawn from REAL data only: part id/label from the primitive kind and parameters, scale, units MM, projection 3RD ANGLE, and rev/author only if known (otherwise "—").
- Small vertical tool palette at top-left. Include only tools that map to real functions (for example orbit/select, reset view, toggle dims, section). No dead buttons.
- 2D/3D mode pill (floating, subtle): "✎ 2D Pencil Vector | ⬡ 3D Clay", bound to renderMode.
## SceneCanvas / viewer
- `pencil2d`: cream/vellum-coloured faces (nearly flat, no specular), graphite edges via EdgesGeometry (2B ink, crisp), hidden edges dashed in 2H (a second depth-func pass or LineDashedMaterial), and an orthographic-feeling camera if feasible.
- `clay3d`: matte warm clay (MeshLambert/Standard, roughness ~1) with soft hemisphere light plus graphite edge overlay. Scene background is transparent so the vellum sheet shows through. Grid and axes are drawn in pencil colours. Re-theme in slate mode via CSS vars read at mount and on theme change.
- Dims: redline (#C84B31) dimension callouts derived from REAL bounds (width/height/depth in mm, mono font), via an SVG/HTML overlay projected from 3D, or Three.js lines plus HTML labels. Section A–A cutting-plane line in redline when showSection is on.
- Dispose geometries and materials properly, keep the existing lifecycle tests green, and support `prefers-reduced-motion`.
## AuxView (right top, ~250px)
Tabs: Section A–A | Ortho 3-View | Tolerance.
- Section: an SVG cross-section computed from the real primitive kind and parameters (box: rectangle; cylinder: rectangle or circle depending on the cut; sphere: circle; anything else: honest "section unavailable for this primitive"). Cut faces get 45° ANSI hatching (SVG pattern) plus redline dimension arrows with real values.
- Ortho: front/top/right SVG projections from real bounds with first/third-angle labels.
- Tolerance: there is no tolerance data in the backend today, so show an honest empty state explaining that. Do not invent values.
## Tests and gates
Unit tests for the section/ortho geometry math and the AuxView tab behaviour, plus a DraftingSheet render test. `npm run typecheck && npm test && npm run build` must be green (if a failure is in the P2b cell's files, re-run once before reacting).
Write `REPORT.md` in this cell folder: files changed, gate counts, gaps, and anything not backed by real data.
