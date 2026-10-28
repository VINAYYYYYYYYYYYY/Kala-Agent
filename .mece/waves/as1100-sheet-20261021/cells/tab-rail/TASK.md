# Cell tab-rail (Phase B): the #13a paper-roll assembly tabs above the border, pinned sub-tabs and the primitive tab

Read `../../WAVE.md` in full: D1, D3, D4, D11, D12, the contract and the rules. Read `../../SPEC.md` § 4B.1. Read `../sheet-shell/REPORT.md` for the frozen props and `state/scriptBuild.ts`. The old design-tab strip lives in `components/drafting/DraftingSheet.tsx`; canvas-3d owns that file and removes it there. The old `components/inspector/**` is yours.

## Do
1. **`TabRail.tsx`** (in the 40 px top margin; the tabs sit on the outer rule like physical index tabs).
   - **One tab per design in the store's LRU, in LRU order:**
     - the label truncated, with a tooltip;
     - a part-count badge `[n]` (n = `component_count`);
     - a chevron;
     - × calls `closeDesign(designId)` (the `design/close` action, WAVE amendment 2). It never deletes the design on the backend.
   - The active tab is raised and joined to the border line.
   - **PRIMITIVE tab (D3):** shown when no design is active, labelled from the real parameters (e.g. `BOX 40×20×30`).
   - **`[+ New Sheet]`** at the end: `clearActiveDesign()`, back to the primitive sheet.
   - The rail scrolls horizontally when full, with keyboard arrows between tabs (a tablist with roving tabindex).
2. **Paper-roll dropdown.** The badge or chevron unfurls a vertical paper roll:
   - an unroll animation with `cubic-bezier(0.22, 1, 0.36, 1)`;
   - paper-curl shading and horizontal ruled lines;
   - `prefers-reduced-motion` → no animation.
   - **Design tabs:** list the real component hierarchy from `design.script.components[].name`, read defensively. Mark which are fused and which are placed (from `assembly`). Show each component's step count.
   - **The PRIMITIVE tab:** the roll holds today's parametric inspector (kind switch, vernier sliders, reset) from `components/inspector/**`, working exactly as today.
3. **Pinned sub-tabs (D12).** Clicking a component pins a mini sub-tab under its parent (`└─ [ Threaded Spindle × ]`); several can sit side by side.
   - Pinning dispatches `subTab/pin` and builds the component alone: `buildScript` with `{summary, components:[component], assembly:[{op:'place', component:name}]}` and the parent's provider, `activate:false`. On result: `subTab/built`. On failure: `subTab/error`, with the backend message.
   - Clicking a ready sub-tab → `activateDesign(childDesignId)` + `subTab/activate {key}`.
   - Clicking the parent tab → activate the parent + `subTab/activate null`.
   - A building sub-tab shows progress. An error sub-tab shows the message and Retry.
   - Dedupe: pinning the same key twice must not rebuild.
4. **Tests:**
   - tabs from LRU order, badge counts, the active state, the primitive tab label, New Sheet;
   - roving-tabindex keys;
   - the paper roll listing from a fixture script (malformed script → an honest empty list);
   - the inspector inside the primitive roll (carry over today's inspector tests);
   - the pin → build → ready flow and the error/retry flow (mock `scriptBuild`); the exact single-component script; dedupe;
   - sub-tab and parent activation.

## Own
- `studio/src/components/tabs/**`, `studio/src/components/inspector/**`.
- The new `studio/src/styles/tabs.css`, and the tests of these.
- The store, types, AppShell and `scriptBuild.ts` are frozen: use "Requests" in the REPORT.

## Gates
`npm run typecheck`, `npm test`, `npm run build` in `studio/`. Other cells edit concurrently: if a failure is in a file you do not own, re-run once, then report it. No commits. Clean up /tmp. Write `REPORT.md`.
