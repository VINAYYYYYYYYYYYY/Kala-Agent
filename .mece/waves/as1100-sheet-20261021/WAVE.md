# Wave: the AS 1100 Living Drafting Sheet (Studio UI redesign), 2026-10-21

**Superseded: the user requested reverting all proposed UI changes without a commit.** See `../ui-revert-20261021/`. This record is historical; do not reapply the redesign.

## User request
The user sent three annotated screenshots (`refs/*.jpg`, red numbered regions) and the redesign brief `SPEC.md` ("Kala — Master UI Redesign Prompt"), with: "same use mini max m3flash and deepseek v4.1 flash as subagents". Opus orchestrates; OpenCode workers implement.

`SPEC.md` is the design source. This file decides where the spec meets real data, and it wins where the two differ.

## Region legend (the red numbers in `refs/`)
- #1 title block at the bottom of the sheet.
- #2 floating "HLR · Preliminary…" paragraph.
- #3 bottom-bar readout (ENV/VOL/AREA/SCALE).
- #4 bottom-right Export / FreeCAD / Options buttons.
- #5 Agent & Kernel journal (right bottom).
- #6 Section A-A aux view (right top).
- #7 three-view ortho inset on the sheet.
- #8 floating "MEASURED ENVELOPE / ISO" text.
- #9 left prompt box.
- #10 left active-geometry card and checks.
- #11 template preset buttons.
- #12 top-left branding header.
- #13 design tabs and the tool palette.
- #14 "2D Pencil Vector / 3D Clay" toggle.
- #15 inline API-key drawer in the left column.
- #16 the four-card Options modal.
- #17 the modal footer.

## Opus decisions (binding)
- **D1, no fake data** (vellum-ui invariant 2). Every number, stamp, status and name comes from the store or the backend, or shows an explicit empty or unknown state ("—", "not reported", "not connected").
- **D2, no chat column.**
  - Process Trail Dot 1 shows the agent's reply to the latest turn.
  - The full thread lives in the pencil's ↺ Prompt History popover: each prompt, reply, model, duration and failure, and a "show design" link.
- **D3, primitive playground stays.** When no design is active, the tab rail shows a `PRIMITIVE` tab (e.g. `BOX 40×20×30`). Its paper roll holds the existing parametric inspector (kind switch and vernier sliders). "+ New Sheet" clears the active design back to the primitive sheet.
- **D4, no import.** There is no STEP import route, so the tab trigger is "+ New Sheet" only.
- **D5, Feature Edit Popover.**
  - ConstructionScript has no fillet, chamfer or offset, so those buttons do not exist.
  - **Parametric nudge:** list the script steps that have a numeric parameter equal to the hovered dimension (|Δ| ≤ 0.01 mm). Give each a stepper (−1, −0.1, value, +0.1, +1) and an Apply. Apply rebuilds the patched script through `POST /api/studio/design` with no model call; the result opens as a new design.
  - **Delete feature:** only when the match is a `pocket` step.
  - **No match:** say so ("no script step has this dimension"), and leave the micro-prompt.
  - **Scoped micro-prompt:** an edit turn carrying `base_design_id` and `focus` (cell edit-turns).
- **D6, avatars.** Eight SVG woodcut-style medallions: an engraved ring, the engineer's monogram, and one emblem each.
  - Emblems: Tesla, coil; Kalam, launch vehicle; Visvesvaraya, sluice gate; da Vinci, gear; Lovelace, punched card; Brunel, chain-link bridge; Watt, centrifugal governor; Hamilton, guidance-computer listing.
  - No photographs and no drawn likenesses (licensing). The selector shows the name and the discipline line from SPEC.
- **D7, Push to CAD.** Rebuilds the active design's exact `script` in the CAD target chosen in the card. For example, an OpenCascade build goes to FreeCAD to get an FCStd. The result opens as a new design. Disabled, with the reason, when that target is unavailable.
- **D8, CAD Color.** The view is ONE merged mesh, so colour its connected triangle islands (disjoint solids) from a fixed engineering palette. A fused design is one colour. The label says "per solid".
- **D9, scale is honest.**
  - The 3D model view moves to an orthographic camera, so the background grid is a true mm grid parallel to the view plane.
  - The pill reads `GRID = 10 MM` (the real spacing) and `SCALE ≈ 1:N`. N is computed from px-per-mm at the CSS reference pixel (0.2646 mm). The title tooltip says the true ratio depends on the monitor.
  - Never print "1:1" unless N is exactly 1 at that reference.
- **D10, telemetry.**
  - Turns are counted from the chat thread.
  - Tokens are the sum of `ChatResponse.usage`, where the provider reported it. Show "— TOK" when no turn reported, and mark a partial sum "≥".
- **D11, mini-map lenses.**
  - **Part Locator:** the hovered feature's own orthographic view (from `/drawing`) with a vermilion box at `FeatureRef.bbox`. With no hover, show the TOP view of the active design.
  - **Assembly Fit:** the parent assembly's TOP view, ghosted, with an amber box at the active sub-tab component's placed bounds. Those bounds are the component's own measured bounds offset by its `place` x, y, z.
  - **Section A-A:** today's section panel, with plane switch and measured area.
- **D12, sub-tabs.** Pinning a component builds it alone through `POST /api/studio/design`, with the parent's CAD provider:
  - the script is `{summary, components:[that component], assembly:[{op:"place",component:name}]}`;
  - the sub-tab shows building, ready (the real measured result) or error.
- **D13, removed regions:** #2, #3, #4, #7, #8, #9, #10, #11, #12, #15, #16, #17. Their functions move as shown in the table below.
- **D14, themes.** `vellum` is Warm Vellum and `slate` is Blueprint Slate. Same store field, recoloured to the SPEC tokens. The slate theme gets its own blueprint tokens (deep blue paper, light ink).
- **D15, untouched:** `studio/electron/**`, `studio/scripts/**`, `electron-builder.json`, `index.html` CSP. No new runtime npm dependencies.
- **D16, edit turns by default.** When a design is active and the provider is not `local`, the pencil barrel shows an `EDITING ‹label› ×` chip, and the turn is sent with `base_design_id`. Clicking × detaches it, so the next turn drafts a new part.
- **D17, finish modes:** `graphite` (default), `cadColor` and `wire`. Wire is today's wireframe material. The old default and matte materials go away.

## Relocation table (nothing that works today may become unreachable)
| Today | New home | Cell |
|---|---|---|
| Chat composer: send, cancel, elapsed timer, client deadline, error rendering | Pencil prompt and its history popover | pencil-prompt |
| Provider picker, model picker (free-only, reload, count), remote-egress consent | Engineer card § Routing | title-block |
| API keys: OpenAI/OpenRouter save/clear, CLI subscription rows | Engineer card § Routing | title-block |
| CAD target selector (no default; the first design turn must ask) | Engineer card § CAD, plus the pencil's blocker chip that opens it | title-block, pencil-prompt |
| Connection status, reconnect/retry, manual browser connect | Engineer card § CAD bridge & kernel | title-block |
| Primitive STEP/STL export and handoff; design export, downloads, FCStd | Engineer card § Export; Process Trail Dot 3 | title-block, dock |
| Primitive inspector (box, cylinder, sphere; vernier sliders) | PRIMITIVE tab paper roll | tab-rail |
| Template buttons (#11) | removed (SPEC) | sheet-shell |
| Active-geometry card and checks (#10) | Process Trail Dot 2 | dock |
| Section A-A aux and plane switch (#6) | Mini-map Section lens | dock |
| Ortho 3-view aux tab and inset (#7) | 2D Orthographic full sheet | ortho-sheet |
| Tolerance aux tab (declared list) | Process Trail Dot 1 (declared material and tolerances, labelled "declared — not analysed") | dock |
| Journal table, chain stamp, poll error | Process Trail audit footer accordion | dock |
| Bottom-bar toggles (grid, axes, dims, section, reset) | Tool strip | canvas-3d |
| Bottom-bar readout | Process Trail Dot 2 metrology | dock |
| Measure tool | Tool strip caliper | canvas-3d |
| Surface material | Finish toggle (D17) | canvas-3d |
| Theme toggle | Engineer card § Profile | title-block |
| Design tabs (#13) | Tab rail above the border | tab-rail |

## Shell geometry (sheet-shell implements; others fit inside)
- **Viewport frame.** An AS 1100 / ISO 5457 double-ruled border: 1.5 px outer rule, 0.75 px inner rule.
  - Margin ring: 20 px on left, right and bottom; 40 px on top, which carries the tab rail.
  - Zone ticks: `1`–`8` along top and bottom, `A`–`F` along left and right.
  - Four triangular centring arrows at the edge midpoints.
- **Inside the inner rule:** a two-column grid, the drafting field (flex) and the right dock (340 px, flush to the right and bottom rules).
- **Dock rows:** `#6` mini-map 28%, `#5` process trail 48%, `#1` title block 24%. Separated by 1 px ink dividers.
- **Drafting-field overlays:**
  - tool strip at top 16 / left 16;
  - view switcher at top 16 / right 16;
  - pencil prompt centred, 20 px above the bottom rule, `width: min(660px, 60vw)`, 44 px tall;
  - scale pill bottom-left.
- **Graphite string:** a viewport-fixed SVG overlay with `pointer-events: none`, above the canvas and below popovers.
- **Popovers** (engineer card, paper roll, feature edit, history) render in a top layer with focus management.

## Contract (sheet-shell writes it; every other UI cell consumes it and never reshapes it)
Add to `studio/src/types/studio.ts`:
```ts
export type ViewMode = 'model3d' | 'ortho2d';
export type ModelFinish = 'graphite' | 'cadColor' | 'wire';
export type MiniMapLens = 'locator' | 'fit' | 'section';
export type CommandSection = 'profile' | 'routing' | 'cad' | 'export';
export type EngineerAvatarId =
  | 'tesla' | 'kalam' | 'visvesvaraya' | 'davinci'
  | 'lovelace' | 'brunel' | 'watt' | 'hamilton';
export type OrthoViewName = 'top' | 'front' | 'right';
/** One hovered or selected projected vector of the 2D orthographic sheet. */
export interface FeatureRef {
  designId: string;
  view: OrthoViewName;
  hidden: boolean;                 // from DrawingView.hidden
  polylineIndex: number;
  segmentIndex: number | null;     // null: the whole polyline (circle/arc)
  kind: 'segment' | 'circle' | 'arc';
  bbox: [number, number, number, number]; // view-plane mm: minU, minV, maxU, maxV
  lengthMm: number | null;
  diameterMm: number | null;       // circle/arc fit; null when not circular
}
export interface ChatUsage { input_tokens: number | null; output_tokens: number | null; }
export interface SubTab {
  key: string;                     // `${parentDesignId}::${component}`
  parentDesignId: string;
  component: string;
  status: 'building' | 'ready' | 'error';
  childDesignId: string | null;
  error: string | null;
}
// ChatRequest gains:  base_design_id?: string | null;  focus?: string | null;   // focus ≤ 1024 chars
// ChatResponse gains: usage?: ChatUsage | null;   (validated in api/validators.ts; absent → null)
```
- **Store** (`studio/src/state/store.ts`):
  - New fields: `viewMode` ('model3d'), `finish` ('graphite'), `miniMapLens` ('locator'), `hoveredFeature`, `selectedFeature`, `commandCard: {open, section}` ({false, 'routing'}), `promptDraft` (''), `editDetached` (false), `subTabs` ([]) and `activeSubTab` (null). `ViewportPreferences` gains `snapToGrid` (false) and `scaleLock` (false).
  - **Persisted** (localStorage, never secrets, validated on read): `profile: {avatar: EngineerAvatarId, initials: string}` (default `{avatar:'tesla', initials:''}`, key `kala-studio.profile`) and `sheet: {title: string, revision: string}` (default `{'',''}`, key `kala-studio.sheet`).
  - `ChatMessage` gains `usage?: ChatUsage | null`, `baseDesignId?: string | null` and `focus?: string | null`.
  - **Removed**: `renderMode`, `auxView`, `optionsOpen` and `optionsCard`, with their actions and selectors. Migrate every user.
  - New actions: `viewMode/set`, `finish/set`, `miniMap/lens`, `feature/hover`, `feature/select`, `commandCard/open {section?}`, `commandCard/close`, `commandCard/section`, `profile/set {patch}`, `sheet/set {patch}`, `prompt/draft {text}`, `edit/detach {detached}`, `subTab/pin {parentDesignId, component}`, `subTab/built {key, childDesignId}`, `subTab/error {key, message}`, `subTab/unpin {key}` and `subTab/activate {key|null}`. Each has a matching selector.
  - `selectSessionUsage(state)` returns `{turns, inputTokens, outputTokens, reportedTurns}`. The token sums are null when no turn reported.
  - **Routing state (amendment 1).** Today `ChatPanel` keeps provider, model, consent and turn start in local `useState`. The pencil and the engineer card both need them, so they move to the store.
    - `routing: {providerId: string, model: string, allowRemote: boolean}`, default `{'', '', false}`. Consent is never persisted. The per-provider model keeps today's `modelCatalog` persistence.
    - Actions:
      - `routing/provider {providerId}`: clears consent and loads `readStoredModel(providerId) ?? ''`, exactly like today's `onProviderChange`.
      - `routing/model {model}`: persists through `modelCatalog` as today.
      - `routing/consent {allowRemote}`.
    - `ChatState.turnStartedAt: number | null`, set by `sendTurn` and cleared by the turn that owns it.
    - Today's default-provider selection effect moves into a `useRoutingDefaults()` hook in `chatTurn.ts`. AppShell calls it once.
  - **Closing a tab (amendment 2).** A `design/close {designId}` action:
    - removes the design from the LRU and from `order`, and drops its drawing and section cache entries;
    - removes every sub-tab whose parent or child is that design;
    - if it was active, activates the most recently used remaining design, or none.
    - The backend copy is never deleted.
- **Shared helpers** (sheet-shell writes them; others call them):
  - `studio/src/state/chatTurn.ts`: the chat-turn logic moved out of `ChatPanel.tsx`, unchanged in behaviour.
    - `sendTurn({prompt, baseDesignId?, focus?})`, `abortTurn()` and `turnReadiness(state)` → `{ready, blocker: 'connection'|'provider'|'model'|'consent'|'cad-target'|null, message}`.
    - It owns the AbortController, the client deadline, the journal appends and the design activation.
  - `studio/src/state/scriptBuild.ts`: `buildScript({script, provider, label, activate})`. Posts the script to `POST /api/studio/design`, puts the result in the store and returns `DesignResult`, or throws a typed error. Used by Push to CAD, nudge and sub-tabs.
  - `studio/src/components/shell/anchors.ts`: `useAnchorRef(id)` and `useAnchorRect(id)` for `'pencil-tip' | 'trail-port'`. The rect stays live through ResizeObserver plus window resize.
- **Stub components** with FINAL props, mounted by AppShell and owned by the Phase B cells:
  - `components/tabs/TabRail.tsx`
  - `components/drafting/ToolStrip.tsx`
  - `components/drafting/ViewSwitcher.tsx`
  - `components/drafting/ScalePill.tsx`
  - `components/ortho/OrthoSheet.tsx`
  - `components/prompt/PencilPrompt.tsx`
  - `components/prompt/GraphiteString.tsx`
  - `components/dock/MiniMap.tsx`
  - `components/dock/ProcessTrail.tsx`
  - `components/titleblock/TitleBlock.tsx`
  - `components/titleblock/EngineerCard.tsx`
- **Backend chat additions** (edit-turns writes them; the UI cells rely on exactly this):
  - `POST /api/studio/chat` accepts optional `base_design_id` (an existing stored design id) and `focus` (≤ 1024 chars).
  - The response always carries `usage: {input_tokens, output_tokens} | null`.
  - Error codes for edit turns: `design_not_found` (404), `edit_requires_design_turn` (400, sent without `cad_provider`) and `edit_requires_model` (400, the `local` provider).

## Phases and ownership (disjoint)
- **A (parallel).**
  - `sheet-shell` (DeepSeek): tokens, layout, store, contract, helpers and stubs.
  - `edit-turns` (DeepSeek): Rust backend, plus the Python reference only where parity needs it.
- **B (parallel, after A).**
  - `canvas-3d` (DeepSeek)
  - `ortho-sheet` (DeepSeek)
  - `pencil-prompt` (MiniMax)
  - `dock` (MiniMax)
  - `title-block` (MiniMax)
  - `tab-rail` (MiniMax)
- **C.** `qa` (DeepSeek): integration, removing dead components with their coverage moved, a visual and a11y pass, and the gates.
- **D.** Opus: review, gates, live app test, AppImage rebuild and smoke.

Each cell's `TASK.md` lists its owned files. In Phase B, the store, the types and AppShell are FROZEN: if you need a contract change, STOP and write it under "Requests" in your REPORT. Opus decides.

## Rules for every cell
- **User data:** in every shell, `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo`, then point `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME` at a fresh `mktemp -d`. Never touch `/home/v9i/.local/share/kala-agent/**`.
- **Secrets and CLIs:** never read credential, auth or settings files (Kala's or any CLI's). Never run the real `claude`, `codex` or `opencode` CLIs.
- **/tmp:** a small quota tmpfs. `rm -rf` YOUR temp dirs when done. Never touch `/tmp/opencode` or other users' and sessions' dirs.
- **Inherited UI invariants:** `.mece/waves/vellum-ui-20261019/WAVE.md` § Invariants all apply. Above all no fake data, the a11y rules (keyboard reachable, visible focus ring, aria labels on icon buttons, `prefers-reduced-motion` respected) and the security posture.
- **Tests:** add tests for every new component and behaviour. Update a test only where it asserts the old layout. Never delete behavioural coverage without moving it.
- **Gates:** in `studio/`, `npm run typecheck`, `npm test` and `npm run build`, all green. The baseline is about 769 tests; re-measure first.
  - Other cells edit concurrently. If a failure is in a file you do not own, re-run once, then report it; do not fix it.
- **Commits:** none.
- **REPORT.md** in your cell folder: the files changed, the contract consumed or added, gate counts, known gaps, and "Requests" (contract changes you need).

## Phase A outcome (Opus, 2026-10-21)
- sheet-shell landed and was reviewed (813/813 tests). Its frozen stub props are in `cells/sheet-shell/REPORT.md`.
- **Decision on its Request 1:** `sendTurn` keeps `cad-target` soft for the legacy panel. Every NEW caller (pencil, micro-prompt) treats every `turnReadiness` blocker as hard.
- **Follow-up for sheet-shell (in flight):**
  - `buildScript` gains an optional `signal`, and uses a build deadline that covers the backend FreeCAD build deadline (1800 s) plus margin, instead of the 60 s compile deadline.
  - Phase B callers may pass `signal` for cancel buttons.

## Task ids (T3 delegate_task; prefix `node:delegated-task:command%3Amcp%3Adbf6ba7a-a4dc-4907-96fd-856bf448e989%3A`)
- sheet-shell `delegate-task%3Aas1100-sheet-shell-r1` (done). Its follow-up 1 is `thread-send%3Aas1100-sheet-shell-followup1`.
- edit-turns `delegate-task%3Aas1100-edit-turns-r1`
- Phase B, launched 2026-10-21 (suffix `delegate-task%3Aas1100-<cell>-r1`):
  - canvas-3d and ortho-sheet (DeepSeek);
  - pencil-prompt, dock, title-block and tab-rail (MiniMax).
- **edit-turns landed and was reviewed.**
  - Opus re-ran `cargo test --workspace`: 583 passed / 0 failed; with `--all-features`, 584 / 0.
  - The diff adds tests and removes none.
  - Error codes, the base-section bytes and the usage mapping are in `cells/edit-turns/REPORT.md`. One extra code matters to the UI: `base_design_unavailable` (502). The UI treats it like `design_not_found` and detaches the edit chip.
- **pencil-prompt landed and was reviewed.**
  - Its own 47 tests pass. The other failures were in-flight files of other cells.
  - Decisions on its Requests:
    - The starter prompts are region #11 (template buttons) and are REMOVED per SPEC; they are not ported.
    - The `trail-port` anchor is the dock cell's (it is in its brief).
    - The collar route stamp is kept.
- **tab-rail landed and was reviewed; follow-up 1 is done.**
  - Pinned children are hidden from the main tabs, the tab order is stable (first-seen), and the primitive roll has no material row.
  - Suite at that point: 1184/1184.
- **dock landed and was reviewed.**
  - The mini-map has three lenses, including MIN WALL. Its TOP-view u/v mapping is model (X, Y), checked against the backend view bases.
  - The process trail has the status line, the trail-port anchor, three dots, artifact downloads and the audit footer.
  - Its three extra honest status sub-states are accepted.
  - AuxView is left for qa to delete; its coverage-move table is in the dock REPORT.
- **canvas-3d landed and was reviewed** (1185/1185 at that point).
  - The 3D view uses an orthographic camera (zoom = px/mm), with the shader grid, graphite, CAD-colour islands with their honest fallback, the tool strip, the switcher and the pill.
  - For qa:
    - drop the transitional `material`, `renderMode`, `titleBlock` and `units` props from DraftingSheet and AppShell, and retire `RenderMode` / `MaterialKind`;
    - make scale lock govern pinch dolly too (a controls-level hook).
- **ortho-sheet landed and was reviewed** (1185/1185).
  - Follow-up 1 is in flight:
    - delete a pocket together with the sketch it consumes;
    - the focus string uses the real endpoints (or the centre and Ø/R);
    - a Cancel button for nudge rebuilds;
    - the hover badge goes in the top layer.
- **ortho-sheet follow-up 1 landed** (1193/1193): sketch + pocket deletion, real-endpoint focus, Cancel build, and a top-layer badge.
  - Note: closing the popover cancels its rebuild, by design.
- **title-block landed and was reviewed.**
  - Opus gates on the combined tree: typecheck 0, **1193/1193 tests (85 files)**, build 0.
- **Opus live look** (temp-data release backend + headless Chromium, 1440×900 and 1280×800, both themes):
  - The local helper + OpenCascade L-bracket builds verified end to end. Shown with it: the ortho sheet with dimensions, the trail, the title block, the tabs, the graphite string and the engineer card.
  - 10 visual defects went to qa (`cells/qa/TASK.md` § Opus findings; screenshots in `cells/qa/opus-shots/`).
- **Phase C** `qa` launched (DeepSeek): `delegate-task%3Aas1100-qa-r1`.
