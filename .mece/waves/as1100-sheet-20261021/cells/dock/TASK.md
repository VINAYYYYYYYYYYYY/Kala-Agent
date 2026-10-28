# Cell dock (Phase B): the #6 Mini-Map and the #5 Graphite Process & Verification Trail

Read `../../WAVE.md` in full: D1, D2, D11, the relocation table, the contract and the rules. Read `../../SPEC.md` § 4E, 4F and § 5.2. Read `../sheet-shell/REPORT.md` for the frozen props and `components/shell/anchors.ts`. The old `components/drafting/AuxView.tsx`, `components/journal/JournalPanel.tsx`, `components/chat/DesignAgentCard.tsx` and `designCard.ts` hold behaviour that must survive here.

## Do
1. **`MiniMap.tsx` (#6, top of the dock, 28%).** Header micro-tabs `PART LOCATOR`, `ASSEMBLY FIT` and `SECTION A-A` (`miniMapLens`). The lens list is a data array (SPEC § 5.2), with only these three real lenses.
   - **Part Locator:**
     - With `hoveredFeature` (or `selectedFeature`): that feature's own view, from the active design's `/drawing` via the shared design controller, fitted, with a `--accent-redline` box at its `bbox` (+4 px).
     - Without one: the TOP view.
     - With only a primitive: today's silhouettes.
   - **Assembly Fit:**
     - When `activeSubTab` is ready: the PARENT design's TOP view ghosted (`--ink-muted`, about 40%), with an `--accent-amber` box at the child design's measured bounds offset by that component's `place` x and y in the parent script (0 when there is no place op).
     - Verify the TOP view's u/v axis mapping against `hlr.ts`/`draftingGeometry.ts` and the backend drawing code, and write it down.
     - Otherwise an empty state: "Pin a component from the tab's paper roll to see where it fits."
   - **Section A-A:** today's section rendering, moved from `AuxView`: plane switch XY/XZ/YZ, the 45° hatch, the measured area, the primitive profile.
     - Add a real `MIN WALL` callout: the minimum distance between an inner loop and the outer loop, computed from the section loops. Show it only when inner loops exist.
2. **`ProcessTrail.tsx` (#5, middle of the dock, 48%).**
   - **Header status, derived from real state:**
     - in flight → `SCRIBBLING GEOMETRY…` with elapsed time;
     - last turn failed → `FAILED · <code or kind>`;
     - verified, valid B-rep and 1 solid → `VERIFIED · MANIFOLD SOLID`;
     - verified with n solids → `VERIFIED · n SOLIDS`;
     - built but not verified → `BUILT · CHECKS FAILED (k)`;
     - a reply without a design → `REPLY · NO GEOMETRY`;
     - otherwise `IDLE · READY`.
   - **`trail-port` anchor:** register it at the entry point at the top of the milestone trail. The pencil's graphite string ends there.
   - **The dotted milestone trail** (`• ── • ── ◉`). A dot is filled only when its milestone is really achieved; the final ◉ only when verified.
     - **Dot 1, Intent & Script:**
       - the latest assistant reply text (clamped, with expand);
       - the design label;
       - real parameter pills from the script (`n components · m steps · k pockets`);
       - declared material and tolerances, labelled "declared — not analysed";
       - a `</> Script JSON` chip that inspects (pretty JSON in a popover, with copy) and downloads (artifact `script`).
     - **Dot 2, B-Rep & Metrology:**
       - stamps `✓ B-REP VALID` and `✓ SOLIDS: n`, only from passed checks;
       - a collapsible full check list (reuse `designCard.ts`; show failed rows first);
       - a tabular grid of VOLUME (cm³), SURFACE AREA (mm²) and ENVELOPE (X × Y × Z mm), with "measured by <provider version>";
       - with only a primitive, today's local or server preview estimate, labelled "local estimate — not measured" or as the server's.
     - **Dot 3, CAD Artifacts:**
       - pills `↓ STEP (.stp)`, `↓ STL (.stl)`, `↓ FCStd` (when `native_kind`) and `↓ Script`, via `getDesignArtifact` and `saveBlob`, with real byte counts in the receipt journal entry;
       - each artifact's digest with copy;
       - a failed download shows its error.
   - **Collapsible audit footer:** `▸ Audit hash-chain · N entries · Verified | Chain broken @n | Session log`, from the real session journal and the chain status. Expanding shows today's `JournalPanel` table, scrollable inside the dock, keeping the poll-error rendering.
3. **Tests:**
   - each lens, including the empty states and the fit offset maths;
   - the min-wall maths;
   - every header status;
   - the dot fill rules;
   - pills from a fixture script;
   - the stamps only on passed checks;
   - the artifact downloads (mock client);
   - the footer counts and chain label.
   - Carry over the AuxView and JournalPanel behavioural tests.

## Own
- `studio/src/components/dock/**`, `components/journal/**`, `components/drafting/AuxView.tsx` (you may reduce it to re-exports, or leave it for qa to delete).
- The new `studio/src/styles/dock.css`, `styles/journal.css`, and the tests of these.
- You may import from `components/chat/designCard.ts`, but do not edit it.
- The store, types and AppShell are frozen: use "Requests" in the REPORT.

## Gates
`npm run typecheck`, `npm test`, `npm run build` in `studio/`. Other cells edit concurrently: if a failure is in a file you do not own, re-run once, then report it. No commits. Clean up /tmp. Write `REPORT.md`.
