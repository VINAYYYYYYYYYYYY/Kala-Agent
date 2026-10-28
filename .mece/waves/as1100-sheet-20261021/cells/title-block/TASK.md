# Cell title-block (Phase B): the #1 AS 1100 agent title block, the engineer button and the Engineer Command Card

Read `../../WAVE.md` in full: D1, D6, D7, D10, D14, the relocation table, the contract (with amendment 1, routing state) and the rules. Read `../../SPEC.md` § 4G. Read `../sheet-shell/REPORT.md` for the frozen props, `state/scriptBuild.ts` and `chatTurn.ts`. The old `components/options/OptionsDrawer.tsx` (cards 02–04), `components/chat/ChatPanel.tsx` (the provider and model pickers, consent, CAD target) and `components/panels/**` hold behaviour that must survive here.

## Do
1. **`TitleBlock.tsx`** (#1, bottom of the dock, 24%). AS 1100 ruled compartments, 0.75 px cell rules, mono cells, uppercase 0.06 em headers.
   - **`PROJECT // SHEET`:**
     - inline-editable title (Enter or blur saves `sheet/set`, Esc cancels);
     - when empty, the active design label shows as a muted placeholder;
     - an editable revision tag, shown as `REV —` when empty.
   - **`AI ENGINE // MODEL`:**
     - provider label and the honest model label (reuse today's default-model label helpers);
     - a health dot: `● READY` only when the provider record says it is available and configured; otherwise its real reason.
   - **`CAD BRIDGE`:**
     - the selected CAD target's name, version and status dot from the live probe, or "not chosen";
     - `[Push to CAD ↗]` per D7: `buildScript` with the active design's script and the selected target. It is disabled with the reason when there is no design, when the design was already built in that target, or when the target is unavailable. Show progress and the result or error.
   - **`TELEMETRY // TOKENS`:** from `selectSessionUsage`, per D10, e.g. `14,280 TOK · 4 TURNS`, `— TOK · 4 TURNS` or `≥ 1,200 TOK · 4 TURNS`.
   - **`DRAFTER`:** the initials stamp, or "—".
   - **Engineer button:** a 56×56 square in the bottom-right cell, with the selected avatar, a 1.5 px ink border and a tactile press shadow. It opens the card and carries an aria label.
2. **`avatars.tsx` (D6).** Eight SVG woodcut/engraving-style medallions, each 56 px and scalable:
   - a heavy engraved ring with hatching;
   - the monogram;
   - one emblem: Tesla, coil; Kalam, launch vehicle; Visvesvaraya, sluice gate; da Vinci, gear; Lovelace, punched card; Brunel, chain-link bridge; Watt, centrifugal governor; Hamilton, guidance listing.
   - Pure black and white, themed via `currentColor`. No raster images and no likenesses.
   - Export metadata `{id, name, discipline}` with the discipline lines from SPEC.
3. **`EngineerCard.tsx`.** A ruled AS 1100 card anchored above the title block. It has a section nav (`commandCard.section`), is focus-trapped, and returns focus on close; Esc closes.
   - **§ Profile:**
     - the 8-avatar selector (radio group, names and disciplines);
     - drafter initials (1–4 letters, uppercased);
     - the theme toggle, `Warm Vellum` / `Blueprint Slate` (`theme/set`).
   - **§ Routing (API keys and LLM routing):**
     - today's provider picker, model picker (free-only, reload, model count) and the remote-egress consent stamp, now on the store's `routing` (amendment 1);
     - today's API-key rows: OpenAI and OpenRouter save/clear with the honest stored-versus-environment behaviour, and the CLI subscription rows with their real reasons;
     - nothing secret is echoed.
   - **§ CAD bridge & kernel:**
     - today's CAD target selector (no default);
     - the loopback connection status (origin, mode, health, backend version);
     - Reconnect / Retry (`connection/retry`), and the manual browser connect, as card 04 does today;
     - the integrations list with the real FreeCAD/Solid Edge status.
   - **§ Export & sheet standards:**
     - the active design's STEP, STL, FCStd and script downloads (shared helpers);
     - today's primitive export and handoff (card 02) when no design is active;
     - **2D sheet export:** `SVG` and `DXF` of the TOP/FRONT/RIGHT orthographic sheet, generated client-side from the real `/drawing` polylines (visible solid, hidden dashed or on a HIDDEN layer), with a title-block text block (project, revision, design id, units mm, third angle). Put the writers in a pure `sheetExport.ts` (DXF R12 ASCII: LINE/POLYLINE entities, layers VISIBLE/HIDDEN/TITLE).
4. **Tests:**
   - each title-block cell (editable title and revision persist; the placeholder; the health dot; telemetry formats);
   - Push to CAD enable rules and its call (mock `scriptBuild`);
   - the avatar selector and initials validation;
   - the theme toggle;
   - routing: a provider change clears consent; model persistence;
   - key save/clear (moved from today's tests);
   - CAD selector and Reconnect;
   - the SVG and DXF writers (structure, layers, coordinates in mm);
   - card focus trap and Esc.
   - Carry over the OptionsDrawer, ProviderPanel and ModelPicker behavioural tests.

## Own
- `studio/src/components/titleblock/**`.
- `studio/src/components/chat/**` EXCEPT `ChatPanel.tsx`. That includes `ProviderPicker`, `ModelPicker`, `CadTargetSelector`, `DesignAgentCard` and `designCard.ts`; keep `designCard.ts`'s exports stable, because the dock imports them.
- `studio/src/components/panels/**`, `components/export/**`, `components/options/**` (mine it; qa deletes the leftovers).
- `studio/src/modelCatalog.ts`, `providerPolicy.ts`, `saveBlob.ts`.
- The new `styles/titleblock.css`, plus `styles/chat.css` and `styles/models.css`, and the tests of these.
- The store, types, AppShell, `chatTurn.ts` and `scriptBuild.ts` are frozen: use "Requests" in the REPORT.

## Gates
`npm run typecheck`, `npm test`, `npm run build` in `studio/`. Other cells edit concurrently: if a failure is in a file you do not own, re-run once, then report it. No commits. Clean up /tmp. Write `REPORT.md`.
