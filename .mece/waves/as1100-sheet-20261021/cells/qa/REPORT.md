# Cell qa (Phase C) — integration, dead-UI retirement, visual & a11y pass

Status: **complete**. All ten Opus findings are fixed, all five cell requests are
implemented, the dead UI is retired with its coverage moved, and all three gates
are green. No commits. No backend file, no `studio/electron/**`, no
`studio/scripts/**`, no `electron-builder.json` and no `index.html` was touched.

## Gates (in `studio/`)

Every gate ran with a fresh `mktemp -d` for `HOME`, `KALA_AGENT_DATA_DIR`,
`KALA_STUDIO_DATA_DIR` and `XDG_DATA_HOME`, and
`npm_config_cache=/home/v9i/.npm`. `/home/v9i/.local/share/kala-agent` was never
touched, no credential/auth/settings file was read, no real `claude`/`codex`/
`opencode` CLI was run, and no remote model API was called (the `local` provider
only). Every temp dir was removed and both backends were stopped (see
"Live run").

| Gate | Before (baseline) | After |
|---|---|---|
| `npm run typecheck` | pass | **pass** (exit 0) |
| `npm test` | **1193 passed / 85 files** | **1109 passed / 76 files** |
| `npm run build` | pass | **pass** (exit 0; only the pre-existing three.js chunk-size warning) |

The net test change is **−84 tests / −9 files**: 111 tests covered the retired
surfaces (the chat column, a bottom bar, an Options drawer, an aux view), and **27 tests were added** in the new homes (listed under "Deletions &
coverage moves"). No behaviour was dropped without a new home; the two
deliberate exceptions are called out there (the SPEC-removed starter prompts
and the card's mass row, which SPEC § 4F does not carry).

## 1. Relocation audit (WAVE table, row by row)

Every row has a live component and a test that proves the behaviour there. The
last column is the proof; "PT" = `test/ProcessTrail.dom.test.tsx`,
"MM" = `test/MiniMap.dom.test.tsx`, "EC" = `test/engineerCard.dom.test.tsx`,
"PP" = `test/pencilPrompt.dom.test.tsx`, "CT" = `test/chatTurn.dom.test.tsx`.

| Today | New home | Proof |
|---|---|---|
| Chat composer: send, cancel, elapsed timer, client deadline, error rendering | PencilPrompt (+ `state/chatTurn.ts`) | PP "sends the prompt and empties the barrel", "sends on Enter, keeps the newline…", "shows the live elapsed clock and the two redline pills", "aborts the turn", "shows the real message of a failed turn…"; CT "reports an operator cancellation…", "reports the client deadline as a timeout…", "carries a provider refusal's reason and available providers…" |
| Provider picker, model picker (free-only, reload, count), remote-egress consent | Engineer card § Routing | EC "carries the provider picker, the model picker and the key rows", "clears the consent stamp when the provider changes", "remembers the model per provider", "shows the egress consent stamp"; `ProviderPicker.dom.test.tsx`, `ModelPicker.dom.test.tsx` |
| API keys: OpenAI/OpenRouter save/clear, CLI subscription rows | Engineer card § Routing | `ProviderPanel.dom.test.tsx` (save/clear, stored vs environment, empty-envelope regression); EC "carries … the key rows" |
| CAD target selector (no default; first design turn asks) | Engineer card § CAD + pencil blocker chip | EC "offers the CAD target selector with no default"; PP "names the CAD target blocker, keeps the nib disabled and opens the CAD section"; `CadTargetSelector.dom.test.tsx` |
| Connection status, reconnect/retry, manual browser connect | Engineer card § CAD bridge & kernel | EC "shows the loopback facts and reconnects"; `ManualConnect.dom.test.tsx` |
| Primitive STEP/STL export and handoff; design export, downloads, FCStd | Engineer card § Export; trail Dot 3 | `designExport.dom.test.tsx`, `ExportPanel.dom.test.tsx`, PT "offers the STEP, the STL, the native file and the script", "downloads an artefact and records the real byte count" |
| Primitive inspector (box, cylinder, sphere; vernier sliders) | PRIMITIVE tab paper roll | `PaperRoll.dom.test.tsx` "the primitive roll carries the inspector", `VernierSlider.dom.test.tsx` |
| Template buttons (#11) | **removed (SPEC)** | n/a — removal is the decision; no surviving caller (verified by the live DOM sweep) |
| Active-geometry card and checks (#10) | Process trail Dot 2 | PT "prints the stamps only from a passed check", "lists the checks with the failures first", "shows the measured grid and names the provider", "marks a not-reported check as neither passed nor failed" |
| Section A-A aux and plane switch (#6) | Mini-map Section lens | MM "draws the hatched cut with the measured area and the loop counts", "writes the chosen plane to the one shared cutting plane", "shows the real error and retries the same plane" |
| Ortho 3-view aux tab and inset (#7) | 2D Orthographic full sheet | `OrthoSheet.dom.test.tsx`, `orthoLayout.test.ts`, `orthoFeatures.test.ts` |
| Tolerance aux tab (declared list) | Trail Dot 1, labelled "declared — not analysed" | PT "lists the declared material and tolerances, labelled as declared" |
| Journal table, chain stamp, poll error | Trail audit footer accordion | PT "shows the ledger when it is expanded, with its poll-error note", "names the verified chain…", "names the first broken sequence…"; `journalLedger.dom.test.tsx` |
| Bottom-bar toggles (grid, axes, dims, section, reset) | Tool strip | `ToolStrip.dom.test.tsx` (every button, `aria-pressed`, reset token, grid/axes/dims/section) |
| Bottom-bar readout (ENV/VOL/AREA/SCALE) | Trail Dot 2 metrology | PT "shows the measured grid and names the provider", "prints an em dash for every measurement…", "keeps the full reported precision in the cell titles" |
| Measure tool | Tool strip caliper | `MeasureTool.dom.test.tsx`, `ToolStrip.dom.test.tsx` |
| Surface material | Finish toggle (Graphite / CAD colour / Wire) | `ViewSwitcher.dom.test.tsx` finish sub-toggle; `DraftingSheet.dom.test.tsx` live finish |
| Theme toggle | Engineer card § Profile | EC "switches the sheet theme" |
| Design tabs (#13) | Tab rail above the border | `TabRail.dom.test.tsx`, `PaperRoll.dom.test.tsx`, `tabScript.test.ts` |

**Defects found by the audit: none open.** Two rows had a real gap and were fixed
in this cell: `selectSessionUsage` could not be passed to `useUiState` (Request
11a), and the Engineer card's sheet export did not reuse the drawing cache
(Request 11b).

## 2. Deletions and their coverage moves

### Components / styles deleted

| Deleted | Why | Coverage moved to |
|---|---|---|
| `src/components/chat/ChatPanel.tsx` (573 lines) | retired chat column (D2) | PencilPrompt + `state/chatTurn.ts` + Engineer card; test moves below |
| `src/components/chat/DesignAgentCard.tsx` (309) | the design card lived only in the chat column | Process trail Dots 1–3 (PT), `designCard.ts`, mini-map lens |
| `src/components/bottombar/BottomBar.tsx` (356) + `styles/bottombar.css` | region #3/#4 removed | Tool strip, trail metrology, scale pill |
| `src/components/options/OptionsDrawer.tsx` (410) | region #16/#17 removed | Engineer card § 01–04 (EC), title block footer |
| `src/components/drafting/AuxView.tsx` (874) | region #6/#7 removed | Mini-map lenses, trail Dot 1, OrthoSheet |
| `src/components/panels/SurfaceMaterialControl.tsx` (59) | the finish owns the surface now (D17) | `ViewSwitcher` finish tests |
| `src/styles/chat.css`, `src/styles/bottombar.css` | no importer left | — |
| dead CSS in `drafting.css`, `layout.css`, `studio.css` | selectors of the retired components (203 dead classes measured across the three sheets) | — |

`src/viewer/HlrDrawing.tsx` is unmounted but **deliberately retained** (see
Gaps): its iso/pan behaviours have no home in the full-sheet ortho design, and
deleting it would delete coverage without a move.

### Test files deleted, and where each behaviour now lives

| Deleted test file | Tests | New home |
|---|---|---|
| `ChatPanel.dom.test.tsx` | 18 | send/cancel/clock/errors → PP "sends…", "shows the live elapsed clock…", "aborts the turn", "shows the real message…"; route/consent/model → EC § Routing; CAD target + provider refusal → **new** CT tests "sends the chosen CAD target, and null when no target was chosen" and "carries a provider refusal's reason…"; deadline/cancellation → CT |
| `chatJournal.dom.test.tsx` | 12 | composer keyboard → PP; thread pinning/newest-first → `promptHistory.dom.test.tsx`; connection stamp → EC § CAD; journal ledger → `journalLedger.dom.test.tsx` (plus **moved** "announces the newest row through a polite live region" and "sticks to the newest row…"); starter tabs → removed with region #11 (SPEC; Opus declined the port) |
| `BottomBar.dom.test.tsx` | 5 | toggles/reset → `ToolStrip.dom.test.tsx`; scale → `ScalePill.dom.test.tsx`; readout → PT metrology |
| `BottomBarDesignReadout.dom.test.tsx` | 8 | measured volume/area/envelope, provider version, em-dash, primitive estimate → PT "shows the measured grid…", **new** "prints an em dash…", **new** "keeps the full reported precision…" |
| `SurfaceMaterial.dom.test.tsx` | 4 | finish toggle → `ViewSwitcher.dom.test.tsx`; the old `viewport.material` field is retired |
| `OptionsDrawer.dom.test.tsx` | 7 | four cards → EC "is a modal dialog with the four sections…"; loopback/versions → EC "shows the loopback facts"; STEP capability → `ExportPanel.dom.test.tsx`; footer title data → `TitleBlock.dom.test.tsx` |
| `AuxView.dom.test.tsx` | 24 | section hatched cut/area/plane/error → MM; tolerances → PT Dot 1; ortho views → `OrthoSheet.dom.test.tsx`; primitive silhouettes → MM Part Locator; design-unavailable honest states → MM + OrthoSheet |
| `DesignAgentCard.dom.test.tsx` | 27 | measured table/checks/stamps/digests/downloads → PT; section pill → MM lens; the turn's card → PT Dot 1; **new** PT "prints an em dash for every measurement…", "marks a not-reported check as neither passed nor failed" |
| `ChatAgentCard.dom.test.tsx` | 4 | provider/model/duration facts → `promptHistory.dom.test.tsx`; **new** "shows a fast local reply in milliseconds…", "puts no provider badge on the operator turn" |
| `previewStatus.dom.test.tsx` | −2 (kept 4) | the two bottom-bar chip assertions retired with the bar; the sheet/inspector refusal and in-flight states remain, plus the live drafting-sheet note |

**27 tests added** (net −84): the new tests named above plus
`projection.test.ts` (free-area fit ×4, pinch lock ×3), `store.test.ts`
(memoised session usage), `dockLogic.test.ts` (`miniMapCaption` ×3),
`ProcessTrail` (provenance, blank-chain audit), `OrthoSheet` (source under the
title), `orthoLayout` (pencil band), `titleBlock` (provider-only route),
`engineerCard` (shared drawing cache), `SceneCanvas` (no floating provenance),
`pencilPrompt` (empty barrel one line, hidden in-flight hint) and
`promptCss` (nib cone, ferrule legibility).

### Dead store helpers / types retired

- `MaterialKind`, `viewport.material`, `finishForMaterial` — removed; `finish`
  is the one appearance state (D17).
- `RenderMode`, `AuxViewKind`, `OptionsCard`, `StudioUnits` — removed from
  `types/studio.ts`.
- `DraftingSheet` no longer accepts `material`, `renderMode`, `titleBlock` or
  `units` (Request 11c); `TitleBlockData` deleted with them.
- `Inspector`'s `showSurfaceMaterial` and the surface-material row removed.

## 3. Removed regions verified live

Ran against the temp-data backend with a real design and the ortho sheet open.
A DOM sweep for every removed region returned **no matches** for
`.chat-panel`, `.bottom-bar`, `.options-drawer`, `.aux-view`, `.studio-zone-left`,
`.studio-zone-right`, `.drafting-title-block`, `.drafting-design-tabs`,
`.drafting-tools`, `.drafting-mode-pill`, `.drafting-sheet-tabs`,
`.options-modal`, `.starter`. The only text left inside `.drafting-field`
(outside its overlays) is the real dimension labels and axis letters:
`W 100.00 mm · H 80.00 mm · D 60.00 mm · +X (WIDTH) · +Y (HEIGHT)`. Regions
#2, #3, #4, #7, #8, #9, #10, #11, #12, #15, #16, #17 do not render.

The live flow used the `local` provider and an **OpenCascade** CAD target:
connect → Engineer card § CAD → OpenCascade → Escape → prompt
"Size an L-bracket 100 x 80 x 60 mm with 8 mm wall" → verified build
(`✓ B-REP VALID · ✓ SOLIDS: 1 · ✓ VERIFIED`, 105.60 cm³, 30112.80 mm²) →
paper roll → pinned sub-tab build (`state: ready`) → mini-map lenses →
ortho hover/popover → STEP download
(`l-bracket-100-x-80-x-60-891ddaaf.step`).

## 4. Opus findings — fixed (before/after screenshots)

All shots are in `cells/qa/shots/`; `before-*` are copied or cropped from
`opus-shots/*.png`, `after-*` were taken in this pass at 1440×900 / 1280×800,
vellum and Blueprint Slate.

| # | Finding | Fix | Before / after |
|---|---|---|---|
| 1 | 3D fit too tight, model under the pencil and field edges | `orthoSheetFit` measures the **free area** (left 64, top 76, right 16, bottom 84 = pencil band), uses **70 %** of it and centres the projection in that area; the orbit target is offset instead of the canvas | `before-design-1440-vellum.png` / `after-design-1440-vellum.png`, `after-design-1280-slate.png` |
| 2 | Graphite reads as flat tan / slate faces near-black | Hatch thresholds rebuilt (a 45–60° face now gets two layers, ≥70° three), ink strength 0.55 → **0.30**, spacing 7 → 9 px; vellum paper tones lightened; slate tokens are blueprint blues (`#5b8cae` lit / `#21445f` shadow) with light ink | `before-finding-02-graphite.png` / `after-finding-02-graphite.png`; `after-design-1440-slate.png` |
| 3 | Provenance label under the view switcher | The floating `.viewer-canvas-source` is gone (no canvas text at all); the provenance is a quiet line in **Process Trail Dot 2**: `view: OpenCascade import (view-only) of the OpenCascade (opt-in) STEP` | `before-finding-03-provenance.png` / `after-finding-03-provenance.png`, `after-finding-03-provenance-dot2.png` |
| 4 | Nib invisible; CLEAR illegible/crowded; barrel text top-aligned | The nib is an SVG wood cone (`--nib-wood`) with grain, a graphite tip (`--nib-graphite`) and an ink outline; disabled now fades the whole cone (0.7) instead of painting it vellum. CLEAR sits in its own paper chip, knurl opacity 0.32; the empty barrel is always one line (placeholder no longer measured) and the input is centred on the axis | `before-finding-04-pencil.png` / `after-finding-04-pencil.png` |
| 5 | Scale pill collides with the ferrule | The stamp moved into the **20 px bottom margin ring** (`left: 4px`, `bottom: 1px`), outside the inner rule, two compact lines; it ends ~85 px from the left, left of zone tick `1` and clear of the pencil at 1280 | `before-finding-05-scale.png` / `after-finding-05-scale.png` |
| 6 | Ortho footer under the pencil; FRONT/RIGHT just above the band | The source line moved **under the title** (`y=46`), the truncation warning to `y=60`, and the ortho layout reserves `padding.bottom = 124` (84 px pencil band + 30 px label line + margin) | `before-ortho-1440-vellum.png` / `after-ortho-1440-vellum.png`, `after-ortho-1280-slate.png` |
| 7 | Mini-map caption truncated | `miniMapCaption()` prints a short, state-derived caption (`TOP VIEW · hover a vector to locate it`, `TOP · hovered segment`, `SECTION A-A · XZ · 6000.00 MM²`, `ASSEMBLY FIT · pin a component`, `BASE_PLATE · ready`); the full lens title stays on the tab's `title` | `before-finding-07-caption.png` / `after-finding-07-caption.png` |
| 8 | Milestone labels truncated; blank band under the header | Compact labels `• INTENT — • CHECKS — • FILES — ◉` (full names in `title` + `.sr-only`); the trail port is a zero-height anchor; body padding-top 2 px | `before-finding-08-trail.png` / `after-finding-08-trail.png` |
| 9 | `0 ENTRIES · VERIFIED` for an empty chain | An empty chain prints `0 ENTRIES` with **no verdict**; a non-empty unverified chain still prints `· Session log` | `before-finding-09-audit.png` / `after-finding-09-audit.png` (fresh backend, 0 entries) |
| 10 | CAD BRIDGE note truncates | The cell prints a one/two-word note (`built here`, `no design`, `no script`, `choose a target`, `probe pending`, `not reported`, `target unavailable`) and the **full reason is the cell's title** and the button's tooltip. The engine route drops a dangling `· —` and prints its provider only; version text fits at 9 px | `before-finding-10-bridge.png` / `after-finding-10-bridge.png` |

Two small layout adjustments were needed by the visual pass, both inside the
cells' designs: Dot 2 now prints the metrology **table** before the collapsible
check list (the table is the reason the dot exists and must stay above the
fold at 1280×800), and the declared material/tolerances are one compact line
each (the check list is one click away). The reply clamp is 180 characters
instead of 220 for the same reason; "Expand" still shows the full reply.

### Request 11 items

1. **Memoised `selectSessionUsage`** — keyed on the `chat.thread` reference; the
   title block now reads it in **one** `useUiState` subscription; tested
   "is memoised on the thread reference…" in `store.test.ts`.
2. **`designController` to `EngineerCard`** — `EngineerCard({ designController })`;
   `SheetExportSection` calls `controller.loadDrawing(designId)` and reads the
   shared `designId|all` cache entry, so a warm ortho sheet is reused (no second
   request); tested in `engineerCard.dom.test.tsx` "reuses the shared drawing
   cache for the sheet export".
3. **Drop transitional props; retire types** — done (see §2).
4. **Scale lock governs the pinch dolly** — `lockedPinchZoom()` + touch
   gesture tracking in `SceneCanvas`: one standard scale step per pinch, never a
   free zoom; 3 unit tests in `projection.test.ts`.
5. **Drop `data-stub`; rewrite shellStubs / AgentWorkspace** — the markers are
   gone from the real components; `shellStubs.dom.test.tsx` now asserts the real
   `.pencil-prompt`, `.ortho-sheet`, mini-map caption, compact trail labels and
   the card dialog; `AgentWorkspace.dom.test.tsx` selects `.pencil-prompt` and
   `.ortho-sheet` and the new `.sheet-scale-slot` mount.

## 5. Accessibility pass

Findings and fixes:

1. **Tab order** — the canvas came before the tool strip and view switcher
   because of DOM order. The overlays now precede `DraftingSheet` in the field
   (absolute positioning keeps the visual order), so the live Tab order is:
   tabs (`tab` → chevron → `+ New Sheet`) → **tool strip (8 buttons)** →
   **view switcher (5)** → **canvas** → **pencil (history → blocker → input →
   Draft)** → **mini-map tab/body** → **trail audit** → **title block (title →
   revision)** → **engineer button** → body. Verified with 25 consecutive Tabs
   on the running app.
2. **Focus rings** — every control gets the 2 px `--focus-ring` from
   `base.css`; the pencil input had `outline: none` and now has an explicit
   `:focus-visible` ring (the bevel remains the resting cue). Checked computed
   `outline-style: solid` on every stop.
3. **Focus traps and returns** — Engineer card: focus lands on the dialog,
   Tab wraps, Escape closes and returns focus to the engineer button (live).
   Prompt history: focus lands in the dialog, Tab wraps, Escape returns to the
   ferrule trigger (live). Feature Edit Popover: takes focus, Escape closes and
   returns focus to the ortho sheet (`ortho-sheet-canvas`, live). All three have
   unit coverage too.
4. **Contrast AA** — the muted construction grey (`#8c857b`, 3.2:1 on vellum)
   was used as **text** in 9 px dock/title/tab lines. Every text rule now uses
   `--ink-secondary` (8.3:1 vellum, 9.6:1 slate); muted ink remains only for
   construction linework, separator glyphs, disabled controls and status marks.
   A new guard in `designSystem.test.ts` fails if a text rule uses
   `--ink-muted`. The token contrast test in `layoutCss.test.ts` still covers
   both themes.
5. **`prefers-reduced-motion`** — emulated live: the paper roll reports
   `animation-name: none`, transitions 0.001 ms; the graphite string becomes a
   static dashed line while a turn runs and nothing afterwards
   (`useReducedMotion` + prompt.css); tab and title-block transitions are
   covered by their own `@media` blocks.
6. **Labels and semantics** — icon buttons keep their aria labels (tool strip,
   lens tabs, artifact downloads, copy, chevrons, engineer button); the
   mini-map is a tablist with Arrow/Home/End; the tab rail is a roving-tabindex
   tablist; the ortho sheet is one focusable group naming arrows/Enter/Escape;
   the empty audit chain no longer claims a verdict; the trail's compact labels
   keep their full names in `.sr-only` text.

## 6. No-fake-data sweep

Grepped the new components for hard-coded numbers, model names, versions,
stamps and telemetry. **No invented value remains**:

- Every measurement, check, stamp, digest, byte count, artifact filename,
  provider/model, version, token count, turn count and journal entry comes from
  `store`/the backend response; missing values print `—`, `not reported`, or an
  explicit no-state.
- Primitive metrology is labelled `local/server estimate — not measured`;
  declared material/tolerances are labelled `declared — not analysed`.
- The provenance line is the real import fact; the scale pill prints an unknown
  dash until a view publishes, and never `1:1` unless exact.
- Only literals found: UI labels, SVG geometry, the standard-scale ladder in a
  tooltip, and the fixed `ISO 2768-m` that the **backend declared** (it is
  rendered from `design.tolerances`, not hard-coded).
- The audit footer's `0 ENTRIES` now carries no verdict, so the sweep's one
  false claim is gone.

## 7. Live run / environment

- Two backend instances were started for screenshots (a long-lived one for the
  main matrix and a dedicated fresh one for the empty-audit shot):
  `KALA_STUDIO_TOKEN=… backend/target/release/kala-studio --port 8793/8794
  --data-dir <temp>/studio --static-dist studio/dist`, with all four user-data
  env vars under `mktemp -d`.
- Both were stopped (`kill <pid>`) and both temp dirs removed, together with
  `/tmp/qa-run`, `/tmp/qa-run2`, `/tmp/qa-crops`, `/tmp/qa-env` and
  `/tmp/strip_dead_css.py`. `/tmp/opencode` was not touched.
- The `local` provider and the OpenCascade target were the only model/kernel
  used; no remote call, no real CLI.

## 8. Files changed

**Source (owned/integration):** `viewer/projection.ts` (free-area fit, pinch
lock), `viewer/SceneCanvas.tsx` (fit, pinch, palette, provenance removed),
`viewer/graphite.ts` (hatching), `styles/{canvas,tokens,layout,prompt,dock,
base,tabs,titleblock,drafting,studio}.css`, `types/studio.ts`,
`state/store.ts`, `components/shell/AppShell.tsx`,
`components/drafting/DraftingSheet.tsx`, `components/prompt/PencilPrompt.tsx`,
`components/ortho/{OrthoSheet.tsx,layout.ts}`, `components/dock/{MiniMap.tsx,
ProcessTrail.tsx,dockLogic.ts}`, `components/titleblock/{TitleBlock.tsx,
format.ts,EngineerCard.tsx,SheetExportSection.tsx}`,
`components/inspector/Inspector.tsx`, `components/tabs/PaperRoll.tsx`.

**Deleted:** the six components, two stylesheets and nine test files in §2.

**Tests modified:** `projection`, `store`, `chatTurn`, `pencilPrompt`,
`promptHistory`, `promptCss`, `dockLogic`, `ProcessTrail`, `OrthoSheet`,
`orthoLayout`, `titleBlock`, `engineerCard`, `SceneCanvas`, `shellStubs`,
`AgentWorkspace`, `layoutCss`, `designSystem`, `previewStatus`, `journalLedger`,
`DraftingSheet`, `MeasureTool`.

**Screenshots:** 44 files in `cells/qa/shots/` — 15 `before-*` files (7 copies
of the Opus look plus the 8 finding crops) and 29 `after-*` files, including the
after matrix:
`after-primitive-1440/1280`, `after-design-1440/1280` in both themes,
`after-ortho-1440/1280`, `after-inflight`, `after-string-success`,
`after-hover`, `after-popover`, `after-card`, `after-paper-roll`,
`after-subtab-building/ready`, `after-minimap-fit/section`,
`after-provenance`, `after-download`.

## 9. Known gaps

1. **`HlrDrawing.tsx` retained.** It is unmounted but its iso view, pan/zoom and
   label-overlap tests have no equivalent in the full-sheet ortho design;
   deleting it would remove coverage without a move. It compiles and is not
   mounted, so it costs no runtime. Its `drafting.css` rules stay with it.
2. The trail body scrolls at 1280×800: the metrology **table** is now above the
   fold (Dot 2 order: stamps → table → checks), but the check row and the
   provenance line sit below it; the audit footer is outside the scroll area.
   The 28/48/24 dock proportions are binding, so this is the honest scroll
   design, not clipping.
3. `DesignAgentCard`'s mass row (`g`/`kg`) and its "design turns need the
   backend" note have no new home. SPEC § 4F omits mass; the backend's refusal
   still reaches the operator through the pencil's error line and the journal.
4. The graphite string's milestone dots sit on the chord while the line is
   still easing taut (they meet after 420 ms). A cosmetic detail of the
   reviewed string animation.
5. `chat-panel-cad-target` class names remain on the CAD target selector (the
   widget now lives in the Engineer card); they are test/stepping hooks, not a
   mounted chat column.
6. The release backend binary predates this UI change (no backend work in this
   cell); the local helper and OpenCascade builds were verified with it.

## 10. Requests

None. All five requests from the cells were implemented in this phase.
