# C0: UI gap pass, backend-independent items (from audit-minimax, triaged by Opus)

Read `../../WAVE.md` (rules and invariants, especially no fake data) and `../audit-minimax/AUDIT.md` (evidence). You own `studio/src/**` and `studio/test/**`. A backend worker is editing `src/kala_agent/**` at the same time; do not touch Python. Items that need the new design API (sheet tabs per design, the jump-to-section pill on design results, the 6-column ledger with digests, the CLI import command) are DEFERRED to a later worker. Do not build them.

## Build (each with tests)
1. **AuxView section label uses real bounds**: pass the mesh `bounds` into `computeSectionView(kind, parameters, bounds)` from `AuxView.tsx` (follow-up from fix-camera/REPORT.md).
2. **SCALE readout** in the BottomBar centre cluster (for example `SCALE 1:1 · FIT` or the real value the sheet uses). It must reflect real state; if the scale is fit-to-view, say FIT rather than inventing a ratio.
3. **Aux header** gets the redline `CUT: 45° ANSI` note, as in Stitch. Keep the footer accurate and shorter.
4. **Material control**: expose the existing `viewport.material` store field (default / matte / wireframe) as a small segmented control in the BottomBar left cluster (or the drawer's card 01 if the bar is crowded at 1280px). It is wired to SceneCanvas, which already supports it. Leave background following the theme, and document that choice.
5. **Measure tool** in the sheet tool palette: click two points on the mesh surface (Three.js raycast against the face mesh), draw a redline leader between them, and show the distance in mm (mono) as a label. Esc or a second tool click clears it. The distance is computed from real intersection points in model mm. Disable the tool when no mesh is present. Unit-test the distance maths and the tool state machine; the DOM test can mock the raycast.
6. **Drafted centre lines**: when Axes is on, add chain-dash (— · —) redline centre lines through the part centre on the sheet, with small `+X (WIDTH)` and `+Y (HEIGHT)` labels in 2H construction grey, projected from the 3D axes the same way the dimension labels are.
7. **Chat agent card**: a badge with the real provider/model from the response (already in `ChatResponse`) and the elapsed time measured client-side from send to reply (`1.2 s`). Show a `local · offline` badge when `offline: true`.

## Not allowed
Select/Mark/Eraser tools (no real function yet; leave them out), fake ratios, fake latency, and any change to electron/, index.html CSP, or packaging.

## Gates
`npm run typecheck && npm test && npm run build` in `studio/`, all green. Then take one screenshot at 1440x900 with the measure tool showing a measurement, using the same backend launch method as `../qa/REPORT.md`, and save it to `shots/`. Write `REPORT.md` here.
