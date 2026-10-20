# C1b: components for design turns (ADR-0009 §7 + Tier 2 of vellum CONSOLIDATED-GAPS)

Start only after C0b (design system) and C1a (data layer) have landed. Read `../../WAVE.md`, ADR-0009 §7, `../b1-api/REPORT.md`, `../c1a-data/REPORT.md` (store contract), `/home/v9i/Projects/Kala-Agent/.mece/waves/vellum-ui-20261019/CONSOLIDATED-GAPS.md` (Tier 2), and `.../vellum-ui-20261019/cells/c0b-system/REPORT.md` (tokens and components you must reuse). You own `studio/src/components/**`, `studio/src/viewer/**`, `studio/src/styles/**` (use the C0b tokens only; no raw px font sizes), AppShell wiring, and the matching tests.

## Decisions made by Opus (binding)
- **2D Pencil with an active design = a true vector drawing.** Render the B1 HLR polylines as SVG on the vellum sheet: the iso view large, plus front/top/right in third-angle arrangement as small views. Visible lines are 2B ink; hidden lines are dashed 2H; section cut faces are hatched. Support pan and zoom (wheel/drag, with reset). Show redline dimension callouts from the MEASURED bounds. This is a static drawing, not the three.js canvas.
  - **3D Clay** keeps the interactive three.js viewport with the design mesh.
  - **Pencil 2D without a design** (plain primitives) keeps the current three.js pencil rendering.
  - Exactly one WebGL canvas exists at most; unmount it while the SVG drawing is shown.
- **Sheet tabs = designs** in the store LRU: label plus a short id, a close button (removes it from the tab strip, and the store keeps the LRU entry), and the active tab bound to `activeDesignId`. A "+" focuses the chat composer with a starter; it never creates fake sheets.

## Build
1. **Agent card**: when a chat turn has a design, render a measured table (volume, area, envelope X/Y/Z, mass only if the material density is DECLARED by the backend; otherwise omit it), checks with ✓/✗ and expected-vs-measured, digest chips (script and STEP sha256, first 8 chars plus copy button), a "B-Rep Verified" Prussian stamp ONLY if `verified`, and a "Section A–A ↗" pill that activates the design and switches AuxView to Section.
2. **Title block**: label, design id (first 8), material (declared), tolerances (declared; e.g. "Ø10 H7 · ISO 2768-m"), scale, units, projection, rev "—". No invented checker or date (use the design's journal timestamp if B1 provides one).
3. **AuxView**:
   - Section: B1 `/section` loops as an SVG with an evenodd path and a 45° hatch pattern, plus the exact area (and "≈" when `area_exact` is false).
   - Ortho: front/top/right from `/drawing`.
   - Tolerance: the declared list, labelled "declared — not analysed".
   - Loading and error states for each.
4. **Journal ledger**: columns SEQ, UTC, TYPE, MESSAGE, DIGEST (short with copy), CHAIN. Local UI entries show "—" and "local". Stamp: "HASH-CHAIN OK" only when chainStatus is verified; "CHAIN BROKEN @ n" in redline when broken; otherwise "SESSION LOG". Newest-first toggle.
5. **Export**: exporting the active design (STEP/STL) shows a receipt with filename, bytes, and sha256 with copy.
6. **BottomBar readout** comes from the active design's measured values with source "OCP measured"; otherwise it keeps the current primitive behaviour.

## Tests and gates
DOM tests for each item, including that the stamps are absent when not backed by data. `npm run typecheck && npm test && npm run build`. Save screenshots of a real local-helper L-bracket turn ("Design a machined aluminum L-bracket for a 40 mm mounting plate, 8 mm wall thickness, with a 10 mm centered bore and 4x M5 corner mounting holes." or a phrasing the B1 parser accepts; see B1 REPORT) at 1440x900 in pencil 2D, clay 3D, the Section tab, and slate. Write `REPORT.md`.

## Added by Opus: backend without design support (binding)
The packaged desktop app may run the native Rust backend (another wave), which reports no `design_available` and has no `/design*` or `/journal*` routes. When `design_available` is not true: hide sheet tabs and the design-only Aux content; show an honest one-line note in the agent card and the Tolerance tab ("Design turns need the Python geometry backend; this backend reports design_available: false"); don't start the journal poller (C1a already gates it); and keep every existing primitive and preview feature working. Add DOM tests for this mode. Never touch `studio/electron/**`, `studio/scripts/**`, or `electron-builder.json`; the Rust wave owns them.
