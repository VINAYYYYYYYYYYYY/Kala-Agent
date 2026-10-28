# Parent live verification

Ran the current release Rust backend and production UI using a throwaway directory under `/tmp/kala-final-live-*`. The stored OpenRouter key was loaded into the child process environment without printing it or copying it to the throwaway settings. The real Kala data directory was not modified.

## Verified flows

- Browser connects through the manual boot gate, using a synthetic per-run bearer token.
- Engineer card selects OpenCascade 7.9.3. Local helper builds `Size an L-bracket 100 x 80 x 60 mm with 8 mm wall` as a verified solid. Measured envelope 100 × 80 × 60 mm; volume 105600 mm³.
- Switched routing to OpenRouter, entered `minimax/minimax-m3.1-flash:free`, granted egress consent through the real card and sent an edit of the original design.
- The first prompt specified width/height/depth in ordinary language. It built a valid 110 × 60 × 80 mm bracket, changing the axis arrangement. This is retained as an observed model limitation, not claimed as an exact-preservation pass.
- A second edit from the original design explicitly changed only X width from 100 to 110 and preserved all Y/Z dimensions, axes, placements and features. The canonical script diff changes only both component sketch widths and the summary. Every other script value is identical. Kernel measured envelope 110 × 80 × 60 mm, volume 116160 mm³; verified one solid.
- Journal records both edit requests against the original base id. Usage: first edit 4216 input + 3362 output tokens; second edit 4223 input + 1434 output tokens. Title block prints `≥ 13,235 TOK · 3 TURNS` because the local first turn has no reported token count.
- Opened the 2D orthographic sheet (46 selectable vectors), selected a 110 mm vector and opened its feature editor. Nudged the base sketch width by +1 mm and Apply rebuilt it locally with no model call. Measured envelope 111 × 80 × 60 mm, volume 116800 mm³, verified one solid.
- Exported SVG (6651 bytes) and DXF (10616 bytes) through the Engineer card, reusing the drawing displayed on the sheet. SVG parses as XML and carries third-angle metadata. DXF has ENTITIES/HIDDEN content and EOF.
- Downloaded STEP through Process Trail: 36247 bytes, ISO-10303-21 format, SHA256 begins `a689ac4b`, matching the visible artifact digest.
- Warm Vellum and Blueprint Slate render graphite hatching. The view is framed clear of the pencil. Scale stamp is in the sheet margin. No `.chat-panel`, `.bottom-bar`, `.options-drawer` or `.aux-view` remains in the live DOM.

## Evidence

`live/original.json`, `live/ambiguous-edit.json`, `live/preserving-edit.json`, `live/journal.json`, `live/nudged-bracket.step`, and the SVG/DXF exports are generated from this run. No credential is present in these artifacts.

Geometry verification confirms the returned solid and metrology. Natural-language interpretation remains model-dependent; the first edit's axis change is evidence of that limit.

## Recovery fix — live backend check

Reloaded the final production bundle `index-Q0Qbhof9.js`, rebuilt the original bracket through the local helper and selected the configured edit route. Temporarily emptied only that bracket's script blob in the throwaway artifact store, retaining a backup. A real edit request was rejected by the backend before any model call with `base_design_unavailable` (502; underlying artifact integrity read returned HTTP 500). The UI retained the actual error and original active design, and removed the edit chip automatically. The throwaway artifact was restored immediately and its full SHA256 rechecked. This independently confirms the recovery fix outside the mocked regression suite.
