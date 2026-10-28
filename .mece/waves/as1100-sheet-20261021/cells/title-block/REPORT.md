# Cell title-block (Phase B) — report

## Gates

| Gate | Result |
|---|---|
| `npm run typecheck` in `studio/` | pass (exit 0) |
| `npm test` in `studio/` | **1193 passed / 1193** in **85 files** (baseline after sheet-shell: 813; the other Phase B cells added the rest) |
| `npm run build` in `studio/` | pass (exit 0; only the pre-existing three.js chunk-size warning) |

Every gate ran with `RUSTUP_HOME=/home/v9i/.rustup`, `CARGO_HOME=/home/v9i/.cargo` and a fresh `mktemp -d` for `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`. No `/home/v9i/.local/share/kala-agent` path was touched, no credential/auth/settings file was read, no real `claude`/`codex`/`opencode` CLI was run. The temp dirs were removed; no commit was made.

Own tests: **68 assertions across 5 files** (`titleBlock` 24, `engineerCard` 18, `avatars` 6, `sheetExport` 15, `ProviderPanel` 5 — one new).

## Files changed

### Created (title block + card)

- `studio/src/components/titleblock/TitleBlock.tsx` — the AS 1100 title block (replaces the Phase A stub). FROZEN PROPS kept: **no props**.
- `studio/src/components/titleblock/EngineerCard.tsx` — the Engineer Command Card (replaces the Phase A stub). FROZEN PROPS kept: **no props**.
- `studio/src/components/titleblock/avatars.tsx` — the eight woodcut medallions (D6): `ENGINEER_AVATARS` (`{id, name, discipline, monogram}`), `avatarById`, `EngineerMedallion`.
- `studio/src/components/titleblock/format.ts` — the pure readings: `formatTokenTelemetry`, `providerHealth`, `cadBridgeView`, `drafterStamp`, `revisionLabel`, `normaliseInitials`, `scriptFromDesign`, `pushToCadState`.
- `studio/src/components/titleblock/sheetExport.ts` — the pure 2D sheet writers: `buildSheetSvg`, `buildSheetDxf`, `layoutSheet`, `scaleLabel`, `sheetFileName`.
- `studio/src/components/titleblock/SheetExportSection.tsx` — the SVG/DXF export control of § 04 (fetches the real `/drawing` and writes the file).
- `studio/src/styles/titleblock.css` — the ruled title block, the 56 px engineer stamp, the card.
- `studio/test/titleBlock.dom.test.tsx`, `studio/test/engineerCard.dom.test.tsx`, `studio/test/avatars.dom.test.tsx`, `studio/test/sheetExport.test.ts`.

### Modified (owned)

- `studio/src/components/export/ExportPanel.tsx` — new optional prop `scope: 'auto' | 'design' | 'primitive'`. `auto` keeps today's behaviour (both halves); the card asks for one half at a time (the design's artefacts **or** the primitive handoff).
- `studio/src/components/panels/ProviderPanel.tsx` — the `/api/studio/settings` envelope no longer erases the provider registry: an envelope with no provider rows keeps the rows the boot gate loaded (an empty list used to knock the whole studio back to the boot gate). The API-key behaviour (save/clear, stored vs. environment) is unchanged.
- `studio/src/components/chat/ProviderPicker.tsx`, `ModelPicker.tsx`, `CadTargetSelector.tsx` — each now imports `styles/models.css` itself: the retired chat column no longer loads it, and these widgets live in the Engineer card now.
- `studio/src/styles/models.css` — the CAD-target rules moved here from `chat.css` and are no longer scoped to `.chat-panel` (the selector lives in the card, which has no chat column).
- `studio/src/styles/chat.css` — the CAD-target block removed (it moved to `models.css`).
- `studio/test/ProviderPanel.dom.test.tsx` — one new test: an empty settings envelope never erases the provider registry.

`components/chat/designCard.ts`, `components/chat/DesignAgentCard.tsx`, `modelCatalog.ts`, `providerPolicy.ts` and `saveBlob.ts` are unchanged: their exports stay stable, as the dock requires.

## Contract consumed

- Store (frozen): `sheet` / `sheet/set`, `profile` / `profile/set`, `theme` / `theme/set`, `routing` / `routing/{provider,model,consent}`, `commandCard` / `commandCard/{open,close,section}`, `cadProvider` / `cad/provider-set`, `connection/retry`, `selectActiveDesign`, `selectCadProvider`, `selectCadProviderRecord`, `selectRouting`, `selectProfile`, `selectServer`, `selectSheetIdentity`, `selectSessionUsage`, `appendJournal`.
- `state/chatTurn.ts` (frozen): `defaultModelLabel` comes from `providerPolicy`; the card uses the store's routing (amendment 1) and never a local copy.
- `state/scriptBuild.ts` (frozen): `buildScript({script, provider, label})` is the only path of **Push to CAD**.
- `types/studio.ts` (frozen): `EngineerAvatarId`, `CommandSection`, `CadProviderRecord`, `ProviderRecord`, `DrawingResponse`, `DesignResult`, `ThemeName`.
- `api/client.ts` (frozen): `getDesignDrawing` for the 2D sheet, `getSettings` / `postSettings` through `ProviderPanel`.
- Frozen props honoured: `TitleBlock` and `EngineerCard` take **no props**; the card is mounted only while `commandCard.open`.

## What the cell does

**Title block (#1, bottom of the dock).** Ruled AS 1100 compartments (0.75 px cell rules, mono cells, uppercase 0.06 em headers):

- `PROJECT // SHEET` — the title and the revision tag are inline-editable (click the cell, Enter or blur saves `sheet/set`, Esc cancels and restores the stored value). While the sheet has no title, the active design's label shows as a muted placeholder; the revision prints `REV —` until one is typed.
- `AI ENGINE // MODEL` — the provider's own name and the honest model label (`defaultModelLabel`, so a CLI says "chosen by …" instead of inventing a model). The health dot prints `● READY` **only** when the backend's record says configured *and* not unavailable; otherwise it prints `● UNAVAILABLE` / `● NOT CONFIGURED` with the backend's own reason.
- `CAD BRIDGE` — the chosen target's name, version and state from the live `/providers/cad` probe, or `not chosen`. `[ Push to CAD ↗ ]` rebuilds the design's exact script in that target through `buildScript`; it is disabled **with the reason** when there is no design, when the design carries no script, when no target was chosen, when the probe has not answered, when the target is unavailable, or when the design was already built in that target. The progress, the result and the error are shown.
- `TELEMETRY // TOKENS` — from `selectSessionUsage`: `14,280 TOK · 4 TURNS`, `— TOK · 4 TURNS`, or `≥ 1,200 TOK · 4 TURNS` when the sum is partial.
- `DRAFTER` — the initials stamp, or `—`.
- The engineer button: a 56 × 56 square in the bottom-right cell with the selected medallion, a 1.5 px ink border, a tactile press shadow and an aria label. It opens the card on § Profile.

**Avatars (D6).** Eight engraved SVG medallions in a 56 × 56 view box (scalable, one colour through `currentColor`, no raster image and no likeness): a heavy ring with 44 hatch ticks, the monogram and one emblem each — Tesla coil, launch vehicle, sluice gate, gear, punched card, chain-link bridge, centrifugal governor, guidance listing. `ENGINEER_AVATARS` carries the SPEC's names and discipline lines.

**Engineer card.** A ruled card in the top layer, anchored above the title block, focus-trapped, Esc-closing, returning the focus to the button that opened it.

- § 01 Profile — the 8-avatar radio group (names + disciplines), the drafter initials (1–4 capitals; an entry without a letter is refused), the `Warm Vellum` / `Blueprint Slate` toggle (`theme/set`).
- § 02 Routing — today's provider picker, model picker (free-only, reload, model count), the remote-egress consent stamp, and today's API-key rows (OpenAI/OpenRouter save/clear with the honest stored-versus-environment behaviour, the CLI subscription rows with their real reasons). A provider change clears the consent; the model keeps its per-provider persistence.
- § 03 CAD bridge & kernel — today's CAD target selector (no default), the loopback facts (origin, mode, health, backend version), `connection/retry`, the manual browser connect (the existing `ConnectionForm`), and the integrations list with the real FreeCAD / Solid Edge status.
- § 04 Export & sheet standards — the active design's STEP, STL, FCStd and script downloads (shared helpers), today's primitive export and handoff when no design is active, and the **2D sheet export**: `SVG` and `DXF` of the TOP/FRONT/RIGHT orthographic sheet, generated client-side from the real `/drawing` polylines (visible solid, hidden dashed in SVG and on the `HIDDEN` layer in DXF) with a title block that names the project, the revision, the design id, the units (mm) and the projection (third angle). The writers are pure (`sheetExport.ts`): DXF R12 ASCII with `LINE`/`POLYLINE` entities and the `VISIBLE`/`HIDDEN`/`TITLE` layers.

## Known gaps

1. The engineer button always opens § Profile. It does not restore the last section.
2. The 2D sheet fits the three views to the sheet and never enlarges them, so the scale statement is always a reduction (`1:2`, `1:2.5`, …) and it is printed as `1:N (FIT)`. The views are centred in their boxes; the model is never re-measured.
3. The DXF is ASCII: a non-ASCII character in the project title becomes `?` in the DXF text (the SVG keeps it).
4. A view the backend did not draw is captioned `TOP — NOT REPORTED` instead of being drawn.
5. Long values in the title block ellipsize; the full text stays in the cell's `title` attribute.
6. The Engineer card fetches the `/drawing` for the sheet export itself (the card takes no props), so it does not share the drawing cache of the 2D orthographic sheet.
7. `ExportPanel` now takes an optional `scope`; the retired `OptionsDrawer` (still unmounted) keeps the old `auto` behaviour. qa deletes the drawer, its file and `OptionsDrawer.dom.test.tsx`.
8. The provider, model and CAD-target widgets now import `models.css` themselves, because the retired chat column was the only thing that loaded it.

## Requests

1. **A memoised session-usage selector.** `selectSessionUsage` builds a fresh object on every call, so it cannot be passed to `useUiState` directly (an unstable snapshot re-renders forever). The title block reads it one primitive at a time (five subscriptions). A `selectSessionUsageParts` (or a memoised `selectSessionUsage`) in the store would make the contract nicer for every future cell.
2. **Optionally: give the Engineer card the `designController` prop** (`{ designController: DesignController | null }`), like `OrthoSheet` and `MiniMap` have. The 2D sheet export would then reuse the drawing cache instead of issuing its own `/drawing` request. The frozen prop list says "no props", so this is a contract change and therefore a request, not a change.
3. **No other contract changes were needed.** The store, the types and `AppShell` were used exactly as sheet-shell froze them.
