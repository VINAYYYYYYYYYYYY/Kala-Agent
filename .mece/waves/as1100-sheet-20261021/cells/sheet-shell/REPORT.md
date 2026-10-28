# Cell sheet-shell (Phase A) — report

## Gates

| Gate | Result |
|---|---|
| `npm run typecheck` in `studio/` | pass (exit 0) |
| `npm test` in `studio/` | **813 passed / 813** in **59 files** (baseline 769) |
| `npm run build` in `studio/` | pass (exit 0; only the pre-existing three.js chunk-size warning) |

Every gate was run with `RUSTUP_HOME=/home/v9i/.rustup`, `CARGO_HOME=/home/v9i/.cargo`, and a fresh `mktemp -d` for `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`. No `/home/v9i/.local/share/kala-agent` path was touched, no credential/auth/settings file was read, no real `claude`/`codex`/`opencode` CLI was run. Temp dirs were removed.

## Files changed

### Created

- `studio/src/components/shell/SheetFrame.tsx` — the decorative AS 1100 double frame, zone ticks 1–8 / A–F, four centring arrows (`aria-hidden`).
- `studio/src/components/shell/anchors.ts` — `useAnchorRef(id)` / `useAnchorRect(id)` for `'pencil-tip' | 'trail-port'`; live through a shared ResizeObserver + `window.resize`.
- `studio/src/state/chatTurn.ts` — the turn machine moved out of `ChatPanel`.
- `studio/src/state/scriptBuild.ts` — `buildScript({script, provider, label, activate})`.
- `studio/src/components/tabs/TabRail.tsx`
- `studio/src/components/drafting/ToolStrip.tsx`
- `studio/src/components/drafting/ViewSwitcher.tsx`
- `studio/src/components/drafting/ScalePill.tsx`
- `studio/src/components/ortho/OrthoSheet.tsx`
- `studio/src/components/prompt/PencilPrompt.tsx`
- `studio/src/components/prompt/GraphiteString.tsx`
- `studio/src/components/dock/MiniMap.tsx`
- `studio/src/components/dock/ProcessTrail.tsx`
- `studio/src/components/titleblock/TitleBlock.tsx`
- `studio/src/components/titleblock/EngineerCard.tsx`
- Tests: `test/SheetFrame.dom.test.tsx`, `test/anchors.dom.test.tsx`, `test/chatTurn.dom.test.tsx`, `test/scriptBuild.test.ts`, `test/shellStubs.dom.test.tsx`, `test/storePersistence.dom.test.ts`.

### Modified (owned)

- `studio/src/types/studio.ts` — the full WAVE contract: `ViewMode`, `ModelFinish`, `MiniMapLens`, `CommandSection`, `EngineerAvatarId` (+ allow-list), `OrthoViewName` (+ allow-list), `FeatureRef`, `ChatUsage`, `SubTab`, `EngineerProfile`, `SheetIdentity`, `CHAT_FOCUS_MAX_CHARS`; `ChatRequest.base_design_id` / `focus`; `ChatResponse.usage`.
- `studio/src/state/store.ts` — new sheet fields/actions/selectors, routing state, persistence with validated reads, `ChatMessage` additions, `ChatState.turnStartedAt`, `selectSessionUsage`, `closeDesign`; removed `renderMode`, `auxView`, `optionsOpen`, `optionsCard` and their actions/selectors.
- `studio/src/state/designs.ts` — `design/close` reducer (LRU + drawing/section cache eviction, active fallback).
- `studio/src/api/validators.ts` — lenient `ChatResponse.usage` validation (absent/invalid → `null`, never throws).
- `studio/src/api/client.ts` — `chatRequestBody` pass-through (base design id validated, focus bounded, fields omitted when unset), used by `postChat`.
- `studio/src/components/shell/AppShell.tsx` — new sheet shell: frame, tab rail slot, drafting field + overlays, dock, graphite string, conditional engineer card.
- `studio/src/components/chat/ChatPanel.tsx` — refactor onto `chatTurn`; routing and turn clock now read/write the store.
- `studio/src/styles/tokens.css` — vellum + blueprint-slate palettes under the contract names, legacy aliases, telemetry/prompt typography tokens, 2% inline-SVG paper grain, sheet geometry tokens.
- `studio/src/styles/base.css` — vellum surface, tabular figures, paper grain.
- `studio/src/styles/layout.css` — the AS 1100 shell geometry, dock rows, overlays, popover layer, stub styles.
- Tests updated where they asserted removed structure: `test/store.test.ts`, `test/designApi.test.ts`, `test/layoutCss.test.ts`, `test/AgentWorkspace.dom.test.tsx`, `test/AppShellMeshStability.dom.test.tsx`, `test/DraftingSheet.dom.test.tsx`, `test/DesignAgentCard.dom.test.tsx`, `test/ChatAgentCard.dom.test.tsx`, `test/AppJournalPollerGate.dom.test.tsx`, `test/ManualConnect.dom.test.tsx`.

### Mechanical migrations outside the owned list (removed store fields only)

- `studio/src/components/drafting/DraftingSheet.tsx` — the old 2D/3D pill now dispatches `viewMode/set` (`pencil2d` → `ortho2d`, `clay3d` → `model3d`); Escape no longer reads `optionsOpen`. Its props and internals are otherwise untouched (canvas-3d owns them).
- `studio/src/components/bottombar/BottomBar.tsx` — `renderMode` → `viewMode` (file stays in the tree, unmounted; qa deletes it).
- `studio/src/components/chat/DesignAgentCard.tsx` — `auxView/set 'section'` → `miniMap/lens 'section'`.

`studio/src/main.tsx`, `studio/src/styles/studio.css`, `studio/src/components/drafting/scaleBus.ts`, `studio/electron/**`, `studio/scripts/**`, `electron-builder.json` and `index.html` were not modified. Backend files modified concurrently by the `edit-turns` cell were not touched.

## Contract as implemented

### Types (`studio/src/types/studio.ts`)

All WAVE types are present exactly: `ViewMode = 'model3d' | 'ortho2d'`, `ModelFinish = 'graphite' | 'cadColor' | 'wire'`, `MiniMapLens = 'locator' | 'fit' | 'section'`, `CommandSection = 'profile' | 'routing' | 'cad' | 'export'`, the eight `EngineerAvatarId`s (exported allow-list `ENGINEER_AVATAR_IDS`), `OrthoViewName` (+ `ORTHO_VIEW_NAMES`), `FeatureRef` (exact field list), `ChatUsage`, `SubTab`, plus `EngineerProfile` and `SheetIdentity` for persistence. `ChatRequest` gained optional `base_design_id` and `focus` (`CHAT_FOCUS_MAX_CHARS = 1024`); `ChatResponse` gained optional `usage`.

The legacy `RenderMode`, `AuxViewKind` and `OptionsCard` types are kept only so the unmounted `SceneCanvas`/`DraftingSheet`/`AuxView`/`OptionsDrawer` still compile until Phase B/C remove them. Nothing new reads them.

### Store

New state: `viewMode` (`model3d`), `finish` (`graphite`), `miniMapLens` (`locator`), `hoveredFeature`, `selectedFeature`, `commandCard {open:false, section:'routing'}`, `promptDraft ''`, `editDetached false`, `subTabs []`, `activeSubTab null`, `routing {providerId:'', model:'', allowRemote:false}`, `profile {avatar:'tesla', initials:''}`, `sheet {title:'', revision:''}`. `ViewportPreferences` gained `snapToGrid` and `scaleLock` (both false).

Persistence, validated field by field on read (corrupt JSON, non-objects, unknown avatars, non-string fields fall back to defaults; text is bounded):

- `kala-studio.profile` → `{avatar, initials}` (initials ≤ 16).
- `kala-studio.sheet` → `{title, revision}` (each ≤ 80).
- `kala-studio.model.<provider>` remains `modelCatalog`'s persistence, written by `routing/model` and by `ModelPicker` as before.
- Consent is never persisted. `routing/provider` clears it and loads `readStoredModel(providerId) ?? ''`.

Actions implemented exactly as the contract names them: `viewMode/set`, `finish/set`, `miniMap/lens`, `feature/hover`, `feature/select`, `commandCard/open {section?}`, `commandCard/close`, `commandCard/section`, `profile/set {patch}`, `sheet/set {patch}`, `prompt/draft {text}`, `edit/detach {detached}`, `subTab/pin {parentDesignId, component}`, `subTab/built {key, childDesignId}`, `subTab/error {key, message}`, `subTab/unpin {key}`, `subTab/activate {key|null}`, `routing/provider`, `routing/model`, `routing/consent`, plus amendment 2's `design/close {designId}`.

Selectors exist for each (`selectViewMode`, `selectFinish`, `selectMiniMapLens`, `selectHoveredFeature`, `selectSelectedFeature`, `selectCommandCard`, `selectPromptDraft`, `selectEditDetached`, `selectSubTabs`, `selectActiveSubTab`, `selectRouting`, `selectProfile`, `selectSheetIdentity`) and `selectSessionUsage(state)` returns `{turns, inputTokens, outputTokens, reportedTurns}`: turns count user rows; each token total is the sum of reported values and stays `null` when no turn reported it; `reportedTurns` counts assistant turns that reported at least one number.

`design/close` drops the design and its LRU entry, drops all `drawing`/`section` cache keys prefixed `${designId}|`, removes every sub-tab whose parent or child is that id, clears `activeSubTab` if it pointed at one, and activates the most recently used remaining design or `null`. It never deletes backend state. `closeDesign(designId)` is exported next to `activateDesign`/`clearActiveDesign`.

`ChatState.turnStartedAt` is set by `chat/send.startedAt` and cleared only by the turn that owns it: `chat/response` / `chat/error` carry `startedAt`, and a superseded turn's reply still joins the thread without clearing the newer turn's clock or in-flight state.

### `api/validators.ts` / `api/client.ts`

`parseChatResponse` validates `usage` leniently: absent, `null`, non-object, a non-integer/negative token, or an object reporting neither number → `usage: null`. It never throws for a missing field. `chatRequestBody` strips `base_design_id`/`focus` when unset or null, validates the id as a 64-hex digest, rejects a focus over 1024 characters, and is the body `postChat` sends.

### `state/chatTurn.ts`

- `turnReadiness(state)` → `{ready, blocker, message}` over `connection` → `provider` (missing, unconfigured, or `available:false`) → `consent` → `model` → `cad-target` (design-capable backend with no CAD target).
- `sendTurn({prompt, baseDesignId?, focus?})` reads the route/consent from `routing`, dispatches the user row with `startedAt`, owns the one AbortController and the client deadline (`chatTurnTimeoutMs(designTurnBudgetSeconds)`), appends the same `You · …`, `POST /api/studio/chat · …` and `Agent · …` journal rows as before, passes `base_design_id`/`focus`/`cad_provider` through, stores reply `usage`, duration, provider/model and design id, and leaves design activation to `postChat`'s validated `design/received`. Timeout is reported as a timeout (never a cancellation); an operator cancel keeps the honest "Cancelled — the backend stopped it" line.
- `abortTurn()`, `useTurnElapsed()`, `formatElapsed`, `chatBadgeText`, `describeChatError`, `useRoutingDefaults()`.
- `sendTurn` treats `cad-target` as a soft blocker: it still sends so the legacy composer can let the backend answer with its provider-choice prompt. The readiness blocker is for the pencil's blocker chip (Phase B). All other blockers refuse the send.

### `state/scriptBuild.ts`

`buildScript({script, provider, label, activate})` reads the ready connection from the store, posts `{provider, script, label?}` through `postDesign`, returns the validated `DesignResult`, and throws `ScriptBuildError {code: 'not-connected' | 'invalid-request' | 'build-failed', cause}`. `activate:false` keeps the result in the LRU but restores the previously active design (or clears it when that design is gone).

### `components/shell/anchors.ts`

`useAnchorRef(id)` publishes an element; `useAnchorRect(id)` returns its viewport `DOMRect` (or `null`), kept live by one shared `ResizeObserver` plus `window.resize`; a removed anchor reports `null`, never a stale point. `resetAnchorsForTests()` is provided for the suite.

### AppShell geometry

`.sheet-shell` → decorative `.sheet-frame` (20 px ring, 40 px top, 1.5 px outer + 0.75 px inner rule) → `.sheet-tab-rail-slot` on the top rule → `.sheet-content` (`minmax(0,1fr) 340px`) → `.drafting-field` with `DraftingSheet`, tool strip at top 16/left 16, view switcher at top 16/right 16, pencil prompt centred 20 px above the bottom rule at `min(660px, 60vw)` × 44 px, scale pill bottom-left 16/16, `OrthoSheet` when `viewMode === 'ortho2d'`; and `.sheet-dock` with rows 28/48/24 separated by 1 px ink rules. `GraphiteString` is a viewport-fixed, pointer-transparent SVG above the canvas; `EngineerCard` renders only while `commandCard.open`. `ChatPanel`, `BottomBar` and `OptionsDrawer` are unmounted but their files/tests remain.

## Stub components — frozen props

| Component | Frozen props |
|---|---|
| `components/tabs/TabRail.tsx` | none; reads store (`designs`, `subTabs`, `activeSubTab`, `sheet`), dispatches `design/activate`, `design/close`, `subTab/*`, `prompt/draft` |
| `components/drafting/ToolStrip.tsx` | none; reads `viewport`, `showSection`, `measure`, `viewResetToken` |
| `components/drafting/ViewSwitcher.tsx` | none; reads `viewMode`, `finish` |
| `components/drafting/ScalePill.tsx` | none; reads `useSheetScale()` from `scaleBus.ts` |
| `components/ortho/OrthoSheet.tsx` | `{ designController: DesignController \| null }` |
| `components/prompt/PencilPrompt.tsx` | none; reads `promptDraft`, `routing`, `chat`, `activeDesign`, `editDetached`, `cadProvider`; calls `chatTurn` |
| `components/prompt/GraphiteString.tsx` | none; reads `chat.inFlight` and the two anchor rects |
| `components/dock/MiniMap.tsx` | `{ designController: DesignController \| null }` |
| `components/dock/ProcessTrail.tsx` | `{ designController: DesignController \| null }` |
| `components/titleblock/TitleBlock.tsx` | none; reads `sheet`, `routing`, `server`, `selectSessionUsage`, `profile`, `theme` |
| `components/titleblock/EngineerCard.tsx` | none; reads `commandCard`, `routing`, `server`, `profile`, `sheet`, `theme`; mounted only when `commandCard.open` |

Each stub renders a labelled empty frame with an explicit `—` (or, for `OrthoSheet`, "No drawing loaded."); no model, design, measurement, provider, token count or stamp is invented. Each has at least one smoke test in `test/shellStubs.dom.test.tsx` (SheetFrame has its own), and the engineer-card stub also verifies initial focus and Escape.

## Known gaps

1. `DraftingSheet` keeps its transitional `renderMode` prop and 2D/3D pill; `AppShell` derives it from `viewMode` (`ortho2d` → `pencil2d`, else `clay3d`). Its own frame/title block and the pill are canvas-3d's to remove.
2. `ViewportPreferences.material` is retained for the transitional `SceneCanvas`/`SurfaceMaterialControl`; `finish` is the live state, and any `viewport/set {material}` also maps onto `finish` (`wireframe` → `wire`, otherwise `graphite`).
3. Legacy `RenderMode`, `AuxViewKind`, `OptionsCard` types remain for the unmounted components until qa deletes them with their files.
4. `EngineerCard` stub has initial focus, `aria-modal`, backdrop close and Escape, but not a full focus trap; title-block Phase B owns the complete card.
5. `DraftingSheet`'s legacy "+" tab still calls `prefillComposer`, which targets the retired `.chat-panel-prompt`; in the new shell it is inert until canvas-3d removes the old tab strip.
6. The pencil prompt / graphite string are idle stubs; no string is drawn and no scribble is simulated.
7. `cad-target` is a readiness blocker only; `sendTurn` treats it as soft so the legacy composer's "send, let the backend ask" behaviour is unchanged. The pencil should use `turnReadiness` to open the CAD section.

## Requests

1. **Confirm the soft `cad-target` rule.** If Phase B wants the pencil to hard-block without a CAD target, that is a one-line change in `sendTurn`, but the legacy `ChatPanel` test "sends cad_provider null when no CAD target has been chosen" would then need an explicit legacy exemption.
2. **Shared route selection is now in the store** (amendment 1), so the pencil and Engineer card need no further contract field. `useRoutingDefaults()` is called by AppShell; `ChatPanel` also calls it so the retained panel keeps working standalone.
3. No other contract changes were needed. The store, types and AppShell are ready to freeze for Phase B.

## Follow-up 1 — long FreeCAD rebuild deadline

Accepted review follow-up. All existing exports and signatures keep working: `postDesign`'s new options argument is a fourth, optional parameter, and `BuildScriptInput.signal` is optional.

### Files changed

- `studio/src/api/client.ts`
  - Added `DESIGN_BUILD_TIMEOUT_MS = 35 * 60_000` (the backend's 1800 s FreeCAD build deadline plus a five-minute margin; it exists only to unstick a wedged backend, and expiry is reported as a timeout).
  - Added `PostDesignOptions { timeoutMs?: number }`. `postDesign(connection, request, signal = null, options = {})` uses `options.timeoutMs ?? DESIGN_COMPILE_TIMEOUT_MS`, so every existing three-argument caller keeps the original 60 s compile budget. A non-positive/non-finite `timeoutMs` is refused before the network.
- `studio/src/state/scriptBuild.ts`
  - `ScriptBuildErrorCode` gained `'cancelled'`.
  - `BuildScriptInput` gained optional `signal?: AbortSignal`, passed to `postDesign`.
  - `buildScript` now calls `postDesign(connection, request, signal ?? null, { timeoutMs: DESIGN_BUILD_TIMEOUT_MS })`.
  - An abort (client `StudioAbortError`, a `DOMException` named `AbortError`, or an already-aborted caller signal) rejects with `ScriptBuildError { code: 'cancelled' }`. A `StudioTimeoutError` stays distinct: `{ code: 'build-failed', message: 'timeout: …' }`, because the backend may still be building.
- `studio/test/scriptBuild.test.ts` — asserts the long deadline and caller signal are passed; abort → `'cancelled'`; timeout → `'build-failed'` with the timeout text.
- `studio/test/designApi.test.ts` — fake-timer proof that an ordinary `postDesign` still times out at `DESIGN_COMPILE_TIMEOUT_MS` (60 s), while a `{ timeoutMs: DESIGN_BUILD_TIMEOUT_MS }` call is still running past that budget and only times out at its own 35-minute deadline.

### Gates after the follow-up

| Gate | Result |
|---|---|
| `npm run typecheck` | pass (exit 0) |
| `npm test` | **817 passed / 817** in 59 files |
| `npm run build` | pass (exit 0) |
