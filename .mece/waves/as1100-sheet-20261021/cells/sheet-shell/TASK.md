# Cell sheet-shell (Phase A): tokens, frame, layout, store contract, shared helpers, stubs

Read `../../WAVE.md` in full first. It is binding: the decisions, the shell geometry, the contract and the rules. `../../SPEC.md` is the visual source.

## Goal
After this cell, the app opens on the new AS 1100 sheet. The frame, the drafting field and the 340 px right dock with its three slots are all in place, every Phase B stub is mounted, and the store, types and helpers carry the FINAL contract. Phase B workers then fill the stubs in parallel without touching shared files.

## Do
1. **Tokens (`styles/tokens.css`).**
   - Vellum palette, exactly from SPEC § 2: `--vellum-bg #F4EFE6`, `--vellum-elevated #FAF6EE`, `--ink-primary #181614`, `--ink-secondary #4A453E`, `--ink-muted #8C857B`, `--accent-redline #C83E2B`, `--accent-amber #D97706`, `--stamp-pass #23533D`, plus a 2% paper grain (CSS/SVG only, no image files).
   - A Blueprint Slate set under `[data-theme="slate"]` with the same token names: deep blueprint blue paper and light ink. Keep text contrast at WCAG AA or better.
   - Typography: JetBrains Mono (tabular-nums) for telemetry and cells; Space Grotesk for prompt and controls. Both fonts are already dependencies.
   - Keep the old token names as aliases while existing components still use them.
2. **Frame (`components/shell/SheetFrame.tsx`, new, plus `styles/layout.css`).** The double-ruled border, the margin ring (40 px top, 20 px elsewhere), zone ticks 1–8 and A–F, and four centring arrows, as in WAVE § Shell geometry. Decorative parts are `aria-hidden`.
3. **AppShell.**
   - Mount, in the WAVE geometry: `TabRail` (in the top margin, sitting on the outer rule), the drafting field (`DraftingSheet` plus the `ToolStrip`, `ViewSwitcher`, `ScalePill` and `PencilPrompt` overlays, and `OrthoSheet` when `viewMode === 'ortho2d'`), the dock (`MiniMap`, `ProcessTrail`, `TitleBlock`), `EngineerCard` (when `commandCard.open`) and `GraphiteString`.
   - Unmount the left column (`ChatPanel`), `BottomBar` and `OptionsDrawer`, but do NOT delete their files or tests. Phase B moves their pieces; qa deletes the leftovers.
   - `DraftingSheet` keeps its current props for now, minus anything removed from the store. Its own frame, title block and floating texts are canvas-3d's job; do not restyle its internals.
4. **Contract.** Implement everything in WAVE § Contract: types, store fields, actions, selectors, persistence with validation on read (corrupt or unknown values fall back to the defaults), and the `ChatMessage` additions. Remove `renderMode`, `auxView`, `optionsOpen` and `optionsCard`, and migrate every user mechanically:
   - `pencil2d` → `viewMode 'model3d'` with finish `graphite`; `clay3d` → `'model3d'`; the old material → finish.
   - The AuxView tab state → `miniMapLens`.
   - `options/open {card}` → `commandCard/open {section}`, with inspector → none (D3), export → 'export', providers → 'routing', connection → 'cad'.
   - Update the affected tests (they assert old structure). Add reducer tests for every new action, the persistence fallbacks, and `selectSessionUsage`.
5. **`api/validators.ts` and `types/studio.ts`.** Add `ChatRequest.base_design_id` and `focus`, and `ChatResponse.usage` (validate the shape; absent or invalid → null, never throw for a missing field, so old backends keep working). `postChat` passes the new fields through only when set.
6. **`state/chatTurn.ts`.** Move the turn logic out of `ChatPanel.tsx` with no behaviour change: send, abort, client deadline from `designTurnBudgetSeconds`, error kinds, journal appends, design activation, elapsed time, and the honest default-model labels.
   - Add `baseDesignId` and `focus` pass-through, store `usage` on the assistant message, and `turnReadiness`.
   - `ChatPanel.tsx` must keep working on top of it (its tests stay green), since it is still in the tree.
   - Unit-test `chatTurn.ts` directly (fake timers for the deadline).
7. **`state/scriptBuild.ts`.** `buildScript` per the WAVE contract, using the existing `postDesign`. Test it with a mocked client.
8. **`components/shell/anchors.ts`.** Per contract, with tests.
9. **Stubs.** Each Phase B component file exists with its FINAL props interface and a minimal honest render: a labelled empty frame and no fake values.
   - `ToolStrip`, `ViewSwitcher` and `ScalePill` need no props (they read the store).
   - `OrthoSheet {designController}`, `MiniMap {designController}`, `ProcessTrail {designController}` and `TabRail {}`.
   - `PencilPrompt {}`, `GraphiteString {}`, `TitleBlock {}` and `EngineerCard {}`.
   - If a stub needs more, pick the props now and document them in the file header: they are frozen afterwards.
   - Give each stub a smoke test.

## Own
`studio/src/styles/tokens.css`, `base.css`, `layout.css`, `studio.css` (shell parts only); `studio/src/types/**`; `studio/src/state/store.ts` and the new `state/chatTurn.ts` and `state/scriptBuild.ts`; `studio/src/api/validators.ts`, `api/client.ts` (the `postChat` pass-through only); `studio/src/components/shell/**`; the new stub files listed above; `studio/src/components/chat/ChatPanel.tsx` (the refactor onto `chatTurn.ts` only); `studio/src/main.tsx`; and the tests of all of these. Mechanical renames in other files are allowed only for the removed store fields. List each one in the REPORT.

## Gates
`npm run typecheck`, `npm test`, `npm run build` in `studio/`. No commits. Clean up your /tmp dirs. Then write `REPORT.md` as described in WAVE § Rules, including each stub's frozen props.
