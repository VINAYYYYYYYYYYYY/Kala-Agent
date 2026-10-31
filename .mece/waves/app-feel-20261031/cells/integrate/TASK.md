# Cell integrate: whole-app tests and visual polish after the shell, composer and settings cells

Read `../../WAVE.md` first. Decisions D1–D9 and the rules still bind.

The three cells are done; their REPORT.md files are in `../shell`, `../composer` and `../settings`. You now own every file under `studio/src` and `studio/test`, but the store contract and `tokens.css` stay unchanged. Opus's after-screenshots are in `../../after/` (`01-idle`, `02-cad-popover`, `03-details-idle`, `04-design`, `05-design-views`, `06-design-activity`). The issues below come from those screenshots.

## Current state
- `npx tsc --noEmit` is clean.
- Full vitest run: 814 passed, 29 failed, all in three whole-app files:
  - `test/AgentWorkspace.dom.test.tsx` (20);
  - `test/layoutCss.test.ts` (7);
  - `test/ManualConnect.dom.test.tsx` (2).

## 1. Whole-app tests (D9)
Rewrite the 29 failing tests for the new structure. Keep each test's behavioural intent and test the same behaviour in its new home:
- **Composer selectors:** they are now in the composer chips and popovers.
- **API keys:** test them via Settings › Models & keys. The chat no longer has a key drawer, so assert that it has none.
- **Active primitive/dimensions/volume:** test them in Details › Properties.
- **AuxView tabs:** test them in Details › Views.
- **Bottom ruler bar → status bar:** it shows the readout, the source and the FreeCAD status. The controls are now tested where they live: Grid, Axes, Dims, Section and Reset on the sheet tool rail, render mode in the top bar, and Export in the top bar (it opens the Export dialog).
- **Journal appenders:** unchanged behaviour; the Activity tab or the store.
- **"OpenCascade geometry estimates in the Options drawer":** in Details › Properties (see 2b).
- **Options drawer → Settings dialog:** Escape and focus return, sections reachable, Reconnect, theme persistence and the Tab trap.
- **4-zone layout → app frame:**
  - the top bar, the chat in the left column and the sheet in the centre;
  - Details closed by default, and when open it holds AuxView and the journal;
  - exactly one SceneCanvas while Settings or Export opens and closes, and while Details toggles;
  - the real connection info in the status bar.
- **`layoutCss.test.ts`:** replace the 4-zone invariants with app-frame invariants. They are:
  - grid rows: top bar 44px, content, status bar 28px;
  - chat 360px and shrinkable, with internal scroll;
  - the sheet column is flexible (`minmax(0,1fr)`);
  - the Details column is 360px, rendered only when open;
  - the compact media queries keep the zones bounded;
  - the Export panel keeps its natural height inside the Export dialog.
- **`ManualConnect`:** the shell now renders with the top bar; update the selectors.

## 2. Visual polish (it must keep D1: same tokens and look)
- **a. Composer chips truncate.** Today they read "Local Deterministic H…" and "CAD: OpenCascade (opt-i…".
  - The CAD chip shows only the short target name (`CAD: OpenCascade`, `CAD: FreeCAD`, `CAD: Solid Edge`, `CAD: choose`). "(opt-in)" and the version stay in the popover and the chip `title`.
  - The CAD chip sizes to its content, and the model chip takes the remaining width.
  - Full text goes in `title`, and the aria-labels are unchanged.
- **b. Details › Properties shows the same numbers three times, and its labels truncate.**
  - Today it shows the Active geometry table, then the Inspector's "OpenCascade estimate (nominal)" card, then "Bounds (mm)". The vernier labels read "WIDTH (SI…", "SUBDIVISI…".
  - Add an `Inspector` prop `showReadouts?: boolean` (default `true`). With `false` it hides the estimate card and the bounds block. DetailsPanel passes `false`.
  - Add a `SURFACE AREA` row to `ActiveGeometrySummary`, with the same source and honesty as the status bar's AREA readout. Never invent a value: show `—` when there is none.
  - Keep the "Closed-form nominal geometry…" sentence once, under the summary, only for primitives.
  - Lay out the vernier sliders in ONE column inside Details, so the full labels fit.
- **c. Details › Views at 360px.**
  - The caption "OFFSET 4.00 mm · 1 OUTER · 0 INNER LOOPS" overlaps "KERNEL AREA". Let it wrap.
  - The sub-tab row clips "CUT: 45° ANSI" at the right edge. Wrap it or move it under the tabs.
- **d. Sheet captions collide with the title block.**
  - With a design, the bottom-right HLR caption ("view: OpenCascade import (view-only) of the OpenCascade (opt-in) STEP") runs under the title block when the sheet is narrower (Details open, or a 1280px window).
  - A footnote ("HLR VECTOR · MEASURED ENVELOPE IN MM · NO SCALE CLAIM") peeks out under the title block's bottom edge.
  - Make the caption block wrap within the space right of the title block, and make the footnote not overlap.
  - Check at 1440×900 and 1280×800, with Details open and closed.
- **e. Activity tab.** Hide the journal footer (origin + backend version) when the journal is inside Details, because the status bar already shows both. Add a prop; keep the default for any other caller.

## 3. Gates (all must pass; report the real numbers)
- In `studio/`:
  - `npx tsc -p tsconfig.json --noEmit` shows 0 errors.
  - `npx vitest run` is ALL green; give the count.
  - `npm run build` is OK. It writes `studio/dist`; that is allowed for this cell only.
- **Screenshots** in `cells/integrate/shots/`, at 1440×900 unless a size is given:
  - `idle`;
  - `design`, after a Local helper + OpenCascade L-bracket turn: CAD chip → OpenCascade → Esc → type `L-bracket 100 x 80 x 60 mm, 8 mm wall` → Draft → wait 15 s;
  - `design-details-properties`;
  - `design-details-views`;
  - `design-1280x800-details-open`;
  - `3d-clay`, after the design turn: the top bar `3D Clay`;
  - `slate-design`;
  - `settings-workspace`;
  - `export`.
- Serve on port 8796 with `tools/serve.sh 8796 <your /tmp build>`. Stop the server and remove your temp dirs at the end.
- `REPORT.md`: the per-file summary, the gate output and the screenshot list, plus anything you could not fix and why.

Rules from WAVE.md apply: temp HOME, no commits, no new deps, no backend changes, never read credential files, never run the real claude/codex/opencode CLIs. One exception to WAVE.md: writing `studio/dist` through `npm run build` is allowed for this cell.
