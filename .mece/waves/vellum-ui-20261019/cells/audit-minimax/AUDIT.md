# Element audit — Kala Studio "Vellum Drafting Board" wave

Auditor: **audit-minimax** (read-only UI element auditor).
Date: 2026-10-19.
Scope: the five Stitch screens assigned in `cells/AUDIT-TASK.md` § Assignment.

Method (per `AUDIT-TASK.md` § Method): every element in `stitch/<screen>/code.html` was enumerated
from the source (plus `screen.png` for visual confirmation), then searched for in
`studio/src/**` and in the QA screenshots `cells/qa/shots/*.png`. Statuses follow the six
categories in the task brief. Nothing outside `cells/audit-minimax/AUDIT.md` was written; no
commit, push, delegation, or credential read was performed. `npm test` / `npm run build` were
**not** re-run (the QA cell already recorded 19 files / 224 tests green at
`cells/qa/REPORT.md:18-19`); the audit is static.

Reference reading order used:
- `cells/AUDIT-TASK.md` (this brief), `WAVE.md` (layout, tokens, invariants).
- `cells/{shell,viewport,chat-journal,qa}/REPORT.md` (what the builders claim they shipped).
- Built source: `studio/src/components/**`, `studio/src/viewer/**`, `studio/src/state/**`,
  `studio/src/api/client.ts`, `studio/src/styles/**`.
- Backend truth: `src/kala_agent/studio/server.py` (routes), `src/kala_agent/studio/schemas.py`,
  `src/kala_agent/studio/integrations/registry.py`.

### A note on the two "workbench" screens (day / night slate)

`WAVE.md:13` names `kala_studio_master_engineering_design_workbench_day_vellum` and
`..._night_slate` as the **day/night theme** references, and `WAVE.md:18-28` fixes the build
target as the 4-zone Vellum board. `cells/qa/REPORT.md:186-188` records the consequence
explicitly: *"The night_slate export is a different 3-panel workbench; per WAVE only its palette
was adopted, so the layout stays the 4-zone vellum board at night."* The two workbench files are
also **byte-identical in structure** (198 visible text nodes each; the only diffs are
`Night`/`Paper` in the theme toggle and `Drafting Vellum`/`Dark Slate` in the `Bg:` readout —
verified by diffing the extracted text of both `code.html` files).

Their rows are therefore audited on the merits: an element is `MISSING-BUILDABLE` when the data
already exists in the store/backend, `MISSING-NEEDS-BACKEND` when honest data does not exist,
`OMITTED-FAKE` for placeholder values, `N/A` for pure mock decoration, and `PARTIAL` when a
built element differs. **Where a row is `MISSING-BUILDABLE` only because `WAVE.md:27` removed the
top header bar / the 3-panel shell, that is called out in the Notes as a WAVE conflict** and the
row is *not* promoted into the "Top gaps" list.

---

# Screen 1 — `kala_studio_the_vellum_drafting_board_master_workspace`

The primary build target (`WAVE.md:11`). `stitch/.../code.html` is 1574 lines;
`screen.png` is 1440×900.

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---|---|---|---|---|
| 1 | Left zone header band | Engineer-pad binding: dashed bottom rule, punch-hole at `left:8px` (`code.html:81-98`) | PARTIAL | `studio/src/components/chat/ChatPanel.tsx:230-258`; `studio/src/styles/chat.css:22-48` | Built draws a stamped double-line card (solid hairline + 0.5px inner rule). The dashed perforation and the punch-hole are not reproduced. |
| 2 | Title stamp `KALA // MECHANICAL DESIGN AGENT` | `code.html:909-911` | PRESENT | `ChatPanel.tsx:232-234`; `chat.css:51-60` | Same wording; the `//` is styled `--redline-ink`. |
| 3 | Connection status pill `LOCAL LOOPBACK` + green dot | `code.html:911-914`, `:111-132` | PARTIAL | `ChatPanel.tsx:235-238`; `chat.css:58-88` | Real state instead of a fixed string: `DESKTOP BRIDGE` / `BROWSER FALLBACK` / `CONNECTING` / `DISCONNECTED`; the dot is coloured per state. Honest, but the label set differs. |
| 4 | Taped starter chips (4) | `[Size L-Bracket] [Size Stepped Shaft] [Housing Envelope] [STEP Handoff]` (`code.html:919-924`) | PRESENT | `ChatPanel.tsx:52-77` (labels), `:271-287` (render); `chat.css:166-218` | Same four labels. Tape strip drawn by `::before` (`chat.css:192-205`). Chips seed the composer and never auto-send. |
| 5 | Ruled note thread (lined paper, scrollable) | `code.html:175-192` | PRESENT | `ChatPanel.tsx:309-348`; `--rule-pattern` `studio/src/styles/tokens.css:40` | Ruled line pattern and bounded scroll present. |
| 6 | User message card | Blue left rule, role dot, `Lead Mechanical Eng`, timestamp (`code.html:928-939`) | PARTIAL | `ChatPanel.tsx:322-335`; `chat.css:325-362` | Role/time/author present, but the author is the literal `You` (no operator identity in the backend) and there is no per-role coloured left rule. |
| 7 | Agent message card | Graphite left rule, green dot, `Kala Agent Kernel`, badge `v1.2 // B-REP`, timestamp (`code.html:942-954`) | PARTIAL | `ChatPanel.tsx:322-335` | The `v1.2 // B-REP` build badge is not rendered — the backend exposes no agent build version (`ChatResponse` = `text/provider/model/offline`, `studio/src/types/studio.ts:202-207`). |
| 8 | Inline `PARAMETRIC SPEC` matrix (5 rows: Envelope, Wall Thickness, Upright Height, Central Bore, Volume/Mass) | `code.html:956-974` | PARTIAL | `ChatPanel.tsx:288-307` (`chat-panel-context-table`); `chat.css:219-283`; data from `studio/src/viewer/localPrimitives.ts` `computeGeometryEstimate` | Replaced by a real 3-row **ACTIVE GEOMETRY** table (dims / envelope / volume) computed from the live parameters. Wall thickness, central bore and material have no field in the primitive contract (`types/studio.ts:77-95`). |
| 9 | Prussian check pill `Section A-A updated in aux view` | `code.html:978-981` | MISSING-BUILDABLE | `AuxView.tsx:110-112`; store `auxView` (`studio/src/state/store.ts:105`, action `auxView/set` `:137`) | The Section A–A view is computed and rendered; only the affordance/affirmation pill is missing. No new data needed. |
| 10 | `sha256:8f3a9e...` chip | `code.html:982-984` | OMITTED-FAKE | — | Fixed placeholder digest. The backend returns no digest for chat turns. `JournalEntry.digest` exists (`types/studio.ts:33-39`) but is only filled from a real export byte count (`ExportPanel.tsx:119-123`). |
| 11 | ISO daemon row: `ISO` badge, `STEP 10303-21 Serialized (14.8 KB)`, green `0.000 DRIFT` | `code.html:989-995` | MISSING-BUILDABLE | `ExportPanel.tsx:117-123` (real filename + byte count + journal digest); route `/api/studio/export` `studio/src/api/client.ts:296` | A real "serialised STEP + size + digest" row already exists after an export, but only in the Export panel / journal, not as a chat-thread status row. `0.000 DRIFT` has no drift metric in the backend. |
| 12 | Composer card (elevated white card, tactile textarea, italic placeholder) | `code.html:999-1004` | PRESENT | `ChatPanel.tsx:420-431`; `chat.css` | Placeholder `Describe geometry, tolerance, or prompt…`. |
| 13 | Composer micro-glyphs: pencil sketch / voice dictation / file attach | `code.html:1009-1019` | N/A | `ChatPanel.tsx:432-447` | No sketch, dictation or attachment-capture capability exists in the store or the backend API surface (`studio/src/api/client.ts:161-318`). Same conclusion as `cells/chat-journal/REPORT.md:70`. |
| 14 | `Remote Egress: Air-Gapped` pill | `code.html:1022-1025` | PARTIAL | `ChatPanel.tsx:388-419`; `REMOTE_PROVIDERS` `ChatPanel.tsx:47` | Replaced by the **real** remote-egress consent stamp, shown only for `openai`/`openrouter`/`codex`; the local route shows the real `loopback only` status. A fixed "Air-Gapped" claim would have been false for remote providers. |
| 15 | Primary button `DRAFT PART GEOMETRY` + `↵ Enter` badge | `code.html:1030-1036` | PRESENT | `ChatPanel.tsx:433-435`; Enter-to-send `ChatPanel.tsx:216-222` | Label `[Draft Part Geometry ↵]`; Enter sends, Shift+Enter newlines, IME composition is respected. |
| 16 | Vellum canvas surface + paper grain | `code.html:1043`, `.paper-grain` `:58-65` | PRESENT | `studio/src/styles/drafting.css:46-55`; grain data URI `drafting.css:19` (`feTurbulence`) | Same role; the Stitch radial-gradient speckle is replaced by an SVG turbulence grain (no CSS gradient, per `WAVE.md:32`). |
| 17 | Drafting corner pins (2: top-left, top-right) | `code.html:1046-1047`, `.drafting-pin` `:887-898` | PARTIAL | `DraftingSheet.tsx:150-153`; `drafting.css:142-155` | Four registration crosses are drawn (all corners) instead of two pushpins. Superset, different look. |
| 18 | Drafting coordinate frame (1.5px outer + 0.5px inner hairline) | `code.html:418-430` | PRESENT | `DraftingSheet.tsx:85`; `drafting.css:57-70` | Double drafted frame present. |
| 19 | Horizontal coordinate ticks `1`–`8` (top and bottom) | `code.html:1051-1056` | PRESENT | `DraftingSheet.tsx:86-103`; `drafting.css:87-96` | 8 ticks per edge; the build also adds boundary tick marks (`DraftingSheet.tsx:122-149`) — a build addition, not a gap. |
| 20 | Vertical coordinate ticks `A`–`F` (left and right) | `code.html:1057-1062` | PRESENT | `DraftingSheet.tsx:104-121`; `drafting.css:97-106` | 6 ticks per edge. |
| 21 | Pencil grid SVG (10px minor, 50px major, hand-ticked `+` at major intersections) | `code.html:1066-1079` | PRESENT | `drafting.css:18` (`--drafting-tick-pattern`), `drafting.css:40` (`--grid-pattern`), `tokens.css:39` | Same 10/50 px geometry and the same `+` tick. |
| 22 | Sheet tab bar: `Sheet 1: L-Bracket` (active, `✕`) + `Sheet 2: Stepped Shaft` + `Sheet 3: Flange Housing` + `+` | `code.html:1082` | PARTIAL | `DraftingSheet.tsx:156-162`; `drafting.css:158-199` | One static tab (`SHEET 01`) plus a `SCALE FIT` label. No sheet list, no close `✕`, no `+` new-sheet button — the store has no multi-sheet model (`studio/src/state/store.ts:94-117`). |
| 23 | Vertical tool palette: 2D/3D pencil toggle, Select, Mark/Redline, Measure/Caliper, Eraser | `code.html:1083-1110` | PARTIAL | `DraftingSheet.tsx:188-238` | The build ships 4 buttons (Reset view, Toggle dimensions, Toggle Section A–A, Toggle grid) — every one wired to a real action, no dead buttons. The 2D/3D toggle moved to the mode pill. Select / Mark / Measure / Eraser have no implementation (see rows 30-31). |
| 24 | Floating mode pill (2D Pencil / 3D tab group) | `code.html:479-509` | PRESENT | `DraftingSheet.tsx:163-185`; `drafting.css:200-246` | `✎ 2D Pencil Vector` / `⬡ 3D Clay`, `aria-pressed`, dispatches `renderMode/set`. |
| 25 | Top-right stamp badge slot (prussian / forest badges) | `code.html:511-539` — the `canvas-top-right-meta` div is **empty** in the HTML | PARTIAL | `studio/src/viewer/SceneCanvas.tsx:191-197` | The Stitch file renders no badge; the build shows a real source line: `Server verified mesh` / `Local preview — pending server verification` / `No mesh`. |
| 26 | Drafted centre lines + `+X (WIDTH)` / `+Y (HEIGHT)` labels | `code.html:1136-1143` | PARTIAL | `SceneCanvas.tsx:339` (AxesHelper), `drafting.css` | A 3D axes helper is drawn and toggled by `showAxes`, but the drafted 2H dash-dot crosshair with `+X (WIDTH)` / `+Y (HEIGHT)` text is not reproduced. |
| 27 | Section cut plane `A ——— A`: redline dash-dot line, arrow flags, `CUTTING PLANE A ——— A` | `code.html:1146-1158` | PARTIAL | `DraftingSheet.tsx:255-271`; `SceneCanvas.tsx` section plane; `studio/src/viewer/draftingGeometry.ts:83,96,110,124` | A–A flags + `CUT A–A · Z = 0.00 mm · mid-plane` and a real 3D cutting plane are drawn. The plane is fixed to the z = 0 mid-plane (no operator-movable cut) and the wording differs. |
| 28 | Drafted isometric part (L-bracket, gusset, 4× M5 holes, bore) | `code.html:1160-1227` | N/A | `types/studio.ts:5` (`PrimitiveKind = 'box' | 'cylinder' | 'sphere'`), `:77-95` | The product's parametric contract is box/cylinder/sphere. Drawing a bracket would be fake geometry; the build honestly renders the active primitive. |
| 29 | Redline dimension callouts (60.00 / 45.00 / 40.00 mm, arrowed dimension lines, extension lines) | `code.html:1229-1261` | PRESENT | `SceneCanvas.tsx:652-712` (Three.js dimension group + projected HTML labels); `drafting.css:358-379` | Real `W / H / D` callouts computed from the live mesh bounds; labels re-project while orbiting. |
| 30 | Bore callout `⌀ 10.00 H7 THRU` + `REAMED FINISH 1.6 Ra` | `code.html:1222-1225` | MISSING-NEEDS-BACKEND | `types/studio.ts:77-95`; `src/kala_agent/studio/schemas.py` | No bore/hole feature, no fit class and no surface-finish field exist in the primitive contract. |
| 31 | Fillet callout `R 4.0 TYP` (leader + arrow) | `code.html:1217-1220` | MISSING-NEEDS-BACKEND | as row 30 | No fillet primitive exists. |
| 32 | Drafted title block, 7 cells (`SUBJECT // PART ID`, `SCALE`, `UNITS`, `PROJECTION`, `TOLERANCE`, `REV`, `AUTHOR`) | `code.html:1268-1297` | PARTIAL | `DraftingSheet.tsx:273-310`; values from `studio/src/components/shell/AppShell.tsx:81-96` | 9 cells (adds `DRAWING` and `SHEET`). `SCALE FIT` (not `1 : 1` — a fit-to-canvas camera would make `1:1` a false claim), `TOLERANCE —`, `REV DRAFT`, `AUTHOR Kala Studio`, `PROJECTION 3RD ANGLE`, `UNITS METRIC (MM)`. |
| 33 | `Alt + Space — Slide-Rule Companion Overlay` watermark | `code.html:1300-1303` | OMITTED-FAKE | — | No companion-overlay feature exists in the brief or the backend; `cells/qa/REPORT.md:189-193` lists the same hint as deliberately not fabricated. |
| 34 | Boxwood ruler bar with hairline measurement markings on the top edge | `code.html:612-630` | PRESENT | `studio/src/styles/layout.css:232-260`; `--boxwood` `tokens.css:33` | 48 px bar on the boxwood token. |
| 35 | Bottom-bar left cluster: `✎ 2D/3D`, `⊞ Grid`, `✛ Axes`, `📏 Dims`, `✂ Cut A-A`, `↺ Reset` | `code.html:1308-1328` | PRESENT | `studio/src/components/bottombar/BottomBar.tsx:56-86` | 7 controls (2D and 3D split out); all dispatch real store actions and are `role="switch"` with `aria-checked`. |
| 36 | Bottom-bar centre cluster: `SCALE: 1:1 METRIC │ ENV: 60×45×40mm │ VOL: 28.16cm³ │ [✓ B-Rep Verified]` | `code.html:1331` | PARTIAL | `BottomBar.tsx:88-97` | Real `ENV` / `VOL` / `AREA` readout with a `server` / `local est.` source badge and an explanatory tooltip. No `SCALE` readout, and no `B-Rep Verified` stamp (see row 45). |
| 37 | Bottom-bar right cluster: `[Export STEP]`, `● FreeCAD Live`, `⚙ Options & Tools` | `code.html:1334-1344` | PARTIAL | `BottomBar.tsx:99-125` | Export STEP opens drawer card 02; FreeCAD shows the **real** registry status (`available_via_neutral_file` → `FreeCAD: neutral`, etc., `BottomBar.tsx:168-173`) — not a live bridge claim; Options opens the drawer (`aria-haspopup="dialog"`). |
| 38 | Aux panel 250px: header with 3 tab pills + redline `CUT: 45° ANSI` | `code.html:1353-1365` | PARTIAL | `studio/src/components/drafting/AuxView.tsx:48-52`, `:82-101`, `:119-138` | Same three tabs (Section A-A / Ortho 3-View / Tolerance), keyboard navigable with `aria-selected` + roving `tabIndex`. The redline `CUT: 45° ANSI` moved from the header into the aux **footer**. |
| 39 | Aux Section A–A drawing: 45° ANSI hatch profile, faint background grid, dash-dot centrelines, redline dimension arrows, `SECTION A—A (1:1)`, `ANSI 45° CAST` | `code.html:1368-1428` | PARTIAL | `AuxView.tsx:143-304` (hatch pattern `:188-197`, grid `:222-227`, centrelines `:229-242`, dims `:256-290`, captions `:292-300`) | Real 45° hatch, grid, centrelines and W/H redline dimensions from live parameters. The profile is the primitive's true section (rectangle / circle), not an L-bracket with a bore void, and the caption reads `ANSI 45° HATCH · REAL PARAMETERS`. |
| 40 | Aux footer note `DETAIL LOUPE: ACTIVE BORE PASS` / `DRIVE: DETERMINISTIC` | `code.html:1430-1433` | OMITTED-FAKE | — | "ACTIVE BORE PASS" describes a bore that does not exist. The build instead shows a real footer (`CUT: 45° ANSI │ PLANE: …` / `PROJ: 3RD ANGLE │ BOUNDS: LIVE` / `SOURCE: — │ VALUES: NONE`, `AuxView.tsx:119-138`). |
| 41 | Journal header `AGENT & KERNEL JOURNAL` + `HASH-CHAIN OK` stamp | `code.html:1441-1446` | PARTIAL | `studio/src/components/journal/JournalPanel.tsx:123-132`, `:221-226`; `AppShell.tsx:140` | Title matches. The stamp renders `SESSION LOG` because the shell passes `hashChainStatus="unknown"`; the `HASH-CHAIN OK` branch is unreachable without a real backend hash-chain report. |
| 42 | Journal ledger rows: coloured tag + time + detail, 12 sample rows (`[plan.created]`, `[partdesign.pad]`, `[ocp.brepgprop]`, `[policy.checkpoint]`, `[freecad.bridge]`, `[audit.merkle]`, `[export.step]`, `[policy.egress]` …) | `code.html:1449-1549` | PARTIAL | `JournalPanel.tsx:141-154`; per-kind tag colours `studio/src/styles/journal.css:149-170`; kinds `types/studio.ts:30` | Real rows are appended for preview/chat/tool/export/connection/system (`state/previewController.ts:132,139-142`; `ChatPanel.tsx:182-183,191,201`; `ExportPanel.tsx:105-127`; `BootGate.tsx:190-289`). The Stitch CAD-event taxonomy has no backend equivalent. |
| 43 | `sha256:…` highlight chip inside a log line | `code.html:1513` | PARTIAL | `JournalPanel.tsx:148-152`; digest source `ExportPanel.tsx:119-123` | Real digest chips are rendered, but only when a real response carries one (export byte count). |
| 44 | Journal audit footer `KEYTAR: IN-MEMORY (LOCKED)` / `SHA-256 STORE: OK` | `code.html:1552-1555` | PARTIAL | `JournalPanel.tsx:170-177` | Real origin (`http://127.0.0.1:8765`) + real `backend v0.1.0` from `server.version`. The `SHA-256 STORE: OK` claim has no backend store, and "KEYTAR" names a mechanism that does not exist in this build. |
| 45 | `[✓ B-Rep Verified]` stamp in the bottom bar | `code.html:1331` | MISSING-NEEDS-BACKEND | `src/kala_agent/studio/server.py:288` (`/preview`) returns `engine` + `warnings` only | A B-Rep verification verdict is a backend capability the studio does not implement; the build correctly shows the mesh **source** instead (`BottomBar.tsx:94-96`). |

**Screen 1 counts:** PRESENT 14 · PARTIAL 21 · MISSING-BUILDABLE 2 · MISSING-NEEDS-BACKEND 3 · OMITTED-FAKE 3 · N/A 2 · **total 45 rows**

---

# Screen 2 — `kala_studio_master_engineering_design_workbench_day_vellum`

A 3-panel workbench shell (module rail / workbench+chat / right inspector) with a top app bar.
`code.html` is a Tailwind mock (41 545 bytes, 198 visible text nodes). See the scope note above:
`WAVE.md:13,18-28` makes this screen a **palette + element reference**, not the build target.

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---|---|---|---|---|
| 1 | Top app bar: gear, `KALA STUDIO` wordmark, `CHAT-FIRST MECHANICAL ENGINEERING AGENT` | `code.html` body head | MISSING-BUILDABLE | `WAVE.md:27` ("There is no top header bar"); title lives in `ChatPanel.tsx:232-234` and `OptionsDrawer.tsx:148` | WAVE conflict: the wave deliberately removed the header bar. The identity exists in the chat stamp and the drawer title. |
| 2 | `LIVE` pill | header | MISSING-BUILDABLE | `ChatPanel.tsx:235-238` (real `Desktop bridge` / `Browser fallback`) | The real connection state is shown in the chat header instead; a fixed "LIVE" would be a false claim. WAVE conflict (no header). |
| 3 | `DESKTOP SHELL` chip | header | MISSING-BUILDABLE | `ConnectionStatus` `studio/src/state/store.ts:43-47` (`mode: 'desktop' \| 'browser-manual'`); `OptionsDrawer.tsx:296` | Real mode string exists and is rendered in Options card 04. |
| 4 | `v0.1.0` version chip | header | PARTIAL | `JournalPanel.tsx:174-176` (`backend v0.1.0`), `OptionsDrawer.tsx:299-301` | Real `server.version` is rendered in the journal footer and Options card 04, not in a header. WAVE conflict. |
| 5 | `NIGHT` / `PAPER` segmented theme toggle | header | PARTIAL | `OptionsDrawer.tsx:249-269` (Vellum/Slate), `store.ts` `theme/set` `:142`; `tokens.css:57-81` | Both themes exist and both palettes are real, but the switch lives in the Options drawer (the wave's "no header bar" rule). |
| 6 | Account / person icon | header | N/A | — | No account or identity concept exists in this product (the studio is a local single-user tool). |
| 7 | Left rail `MODULES // KERNELS` + `3 Active` | left rail | MISSING-BUILDABLE | `store.ts:73-81` (`ServerState.integrations` / `.providers`); `BootGate.tsx:99-106` | The counts are real and already computed at boot. |
| 8 | `[CAD] Parametric kernel · 3 Int.` | left rail | PARTIAL | `src/kala_agent/studio/integrations/registry.py:70-212`; `IntegrationsPanel.tsx:19-46` | The CAD integration registry exists and is rendered in Options card 02 with real statuses, but not as a left module rail. |
| 9 | `[Analysis] FEA/CAE solver · 2 Int.` | left rail | MISSING-NEEDS-BACKEND | `registry.py:152-212` (ANSYS/Abaqus/COMSOL/Nastran/OptiStruct rows exist as **registry entries only**, all `native_api_unavailable`) | The registry lists them; no analysis solver is mounted, so a "solver" module is not honestly available. |
| 10 | `[Providers] API keys & keytar · 2/3 Conf.` | left rail | PARTIAL | `OptionsDrawer.tsx:199-211`; `ProviderPanel` `studio/src/components/panels/ProviderPanel.tsx`; `src/kala_agent/studio/server.py:230` (`/providers`), `:246` (`/settings`) | Real provider records and real credential status exist and are rendered in Options card 03. The "2/3 Conf." count is derivable from `server.providers`. |
| 11 | `INTEGRATIONS` section header + `FreeCAD / SOLIDWORKS / Fusion 360` with coloured status dots | left rail | PARTIAL | `IntegrationsPanel.tsx:8-12, 26-46`; `journal.css`-style per-status classes; `registry.py:70-138` | All 11 registry rows are rendered with real status labels; the traffic-light dot per integration is not used. |
| 12 | `KEYTAR ENCLAVE` card — `127.0.0.1:8765 (desktop shell)` | bottom-left | PARTIAL | `OptionsDrawer.tsx:289-296` (Origin, Mode), `JournalPanel.tsx:170-173` | Real origin and mode are shown; the "Keytar" product name is invented (there is no keytar dependency — credentials live in a 0600 `studio-settings.json`, see `ChatPanel.tsx:262-266`). |
| 13 | Right-rail top tabs `WORKBENCH / EXPORT / ASSISTANT` | right rail | PARTIAL | `OptionsDrawer.tsx:35-40`, `:160-172` | The wave replaced these with 4 drafted Options cards (01 Inspector / 02 Export / 03 Providers / 04 Connection) covering the same ground. WAVE conflict. |
| 14 | Toolbar: `AGENT` / `3D FOCUS` toggle | workbench toolbar | MISSING-NEEDS-BACKEND | `studio/src/viewer/SceneCanvas.tsx` | "3D focus" implies a saved named camera / per-object framing; no camera-persistence concept exists in the store. |
| 15 | Toolbar: `SHAPE: BOX / CYL / SPH` | workbench toolbar | PARTIAL | `Inspector.tsx:149-173` (`PrimitivePicker`); `store.ts:126` `primitive/change` | The same three primitives, but the switch lives in Options card 01 rather than a top toolbar. |
| 16 | Toolbar: `Grid` / `Axes` / `Dims` / `Section Cut` checkboxes | workbench toolbar | PRESENT | `BottomBar.tsx:73-76`; `DraftingSheet.tsx:201-237` | All four toggles exist, in the bottom bar and the sheet tool palette, bound to real state. |
| 17 | Toolbar: `Mat: Wireframe` | workbench toolbar | MISSING-BUILDABLE | `MaterialKind` `store.ts:83`, default `'default'` `store.ts:208`; `SceneCanvas.tsx:414-417` | The material kind exists in the store and the renderer implements it, but **no UI control** writes it (grep: no `viewport/set` with `material` outside `AppShell.tsx:122`). |
| 18 | Toolbar: `Bg: Drafting Vellum` | workbench toolbar | MISSING-BUILDABLE | `BackgroundKind` `store.ts:84`; `AppShell.tsx:76-79` | The store field exists, but the build derives the canvas background from the theme and exposes no control. |
| 19 | Toolbar: `Reset` | workbench toolbar | PRESENT | `BottomBar.tsx:77-85` (`view/reset` → `SceneCanvas.resetView()`) | Real camera re-frame, no canvas remount. |
| 20 | Viewport mode bar `2D ORTHOGRAPHIC ← → 3D ISOMETRIC ORBIT` slider | viewport top | MISSING-BUILDABLE | `SceneCanvas.tsx` camera FOV (24° pencil FOV, `cells/viewport/REPORT.md:107-110,166-168`) | The camera exists and the pencil mode approximates orthographic, but there is no control to choose an orthographic vs. isometric camera. |
| 21 | `Cut A–A: 50% Section Plane` badge | viewport top | PARTIAL | `draftingGeometry.ts:83,96,110,124`; `DraftingSheet.tsx:267-271` | Section A–A exists but the plane is fixed at the z = 0 mid-plane (50 % of the depth for centred primitives); no offset control. |
| 22 | `SERVER VERIFIED MESH` + `B-Rep v7.8` badge | viewport top | PARTIAL | `SceneCanvas.tsx:191-197`; `previewController.ts:139-142` (journal carries the real `engine`) | The source line is real; the engine version is shown in the journal, not as a badge. |
| 23 | Drafting canvas frame, ticks `1`–`8` / `A`–`C` | viewport | PARTIAL | `DraftingSheet.tsx:71-72, 86-121` | The build has the same 1–8 top/bottom ticks and **A–F** (two more rows than the Stitch canvas). |
| 24 | Wireframe part with dimension leaders `t = 8.00 mm`, `20.00 mm (Y)`, `30.00 mm (Z)`, `R 4.00 TYP` | viewport | PARTIAL | `SceneCanvas.tsx:652-712` | Real W/H/D callouts; the wall-thickness and fillet leaders have no data source (rows 30-31 of screen 1). |
| 25 | Axis gizmo bottom-right (`ISO 30° / WGS84 0-beta`) | viewport | OMITTED-FAKE | — | `WGS84 0-beta` is a mock coordinate-system claim; the studio works in plain millimetres (`types/studio.ts:24`). |
| 26 | `SLIDE-RULE COMPANION` overlay (`Alt+Space`): `t_wall`, `fillet_r`, `material`, `safety_f` | floating card | MISSING-NEEDS-BACKEND | `types/studio.ts:77-95`; `studio/src/api/client.ts` | Wall thickness, fillet radius, material and safety factor have no field in the primitive contract and no backend endpoint. |
| 27 | `Quick-Spool STEP to FreeCAD` button | floating card | PARTIAL | `ExportPanel.tsx:98-131` (`postHandoff`); `registry.py:122-131` (FreeCAD = `available_via_neutral_file`) | A real handoff exists, but it lives in Options card 02 and it is a **neutral-file** handoff, never a live "spool" to a running FreeCAD. |
| 28 | Floating tool palette: dims readout, `Reset Cam`, `Section Cut`, `Measure`, `Wireframe` | floating card | PARTIAL | `DraftingSheet.tsx:188-238`; `BottomBar.tsx:56-97` | Reset, Section Cut, Dims and Grid exist. `Measure` (a caliper tool) and a `Wireframe` toggle do not. |
| 29 | Status strip `[BOX] Dimensions: … │ Volume: … │ Status: ISO 10303 READY │ Engine: OpenCASCADE 7.8` | under viewport | PARTIAL | `BottomBar.tsx:88-97`; `Inspector.tsx:184-202`; `previewController.ts:139-142`; `OptionsDrawer.tsx:64-69` | All four facts exist and are real — dims/volume in the bottom bar and inspector, `ISO 10303 READY` in Options card 02 (from `server.stepAvailable`), the real engine string in the journal. They are split across panels instead of one strip. |
| 30 | Shortcuts row: `+ Size an L-bracket`, `+ Size a shaft`, `+ Estimate a housing`, `+ Plan STEP/STL handoff` | under status strip | PRESENT | `ChatPanel.tsx:52-77, 271-287` | Same four starters, rendered as taped chips in the left zone. |
| 31 | Chat: `Lead Mechanical Engineer` card + UTC timestamp | chat | PARTIAL | `ChatPanel.tsx:322-335` | Role/author/time present; the author name is a fixed `You` (no operator identity in the backend). |
| 32 | Chat: `Kala Mechanical Agent` card + `OpenCASCADE v7.8 · gpt-4o-mini · Deterministic` badge + `(2.8s)` duration | chat | PARTIAL | `ChatPanel.tsx:322-335`; `types/studio.ts:202-207` | The engine/model badge and the elapsed time are **not** rendered: `ChatResponse` carries only `text/provider/model/offline`; the real provider/model are known to the request but the response badge is not built. The duration could be measured client-side. |
| 33 | Agent prose with an inline code chip `t = 8.000 mm` | chat | PARTIAL | `ChatPanel.tsx:333` (plain `<p>` text) | The chat renders the raw returned text; markdown/code chips are not parsed. The real geometry facts are available in the ACTIVE GEOMETRY table (`ChatPanel.tsx:288-307`). |
| 34 | `STRUCTURAL CONSTRAINT EVALUATION` block: `PASSED (SF = 2.45)`, Flange Thickness, Mounting Pitch, Max Bending Stress `112.4 MPa < 276.0` | chat | OMITTED-FAKE | `Inspector.tsx:195-199` (explicit disclaimer) | These are mock FEA results. The backend computes no stress, deflection or safety factor; the build states this in the inspector disclaimer and in the drawer footer stamp `NO CERTIFIED FEA RESULT` (`OptionsDrawer.tsx:231`). |
| 35 | Agent action buttons `Apply 8mm Web Gusset` / `Export to .STEP` | chat | PARTIAL | `ExportPanel.tsx:181-188` (real Download button) | `Export to .STEP` exists (Options card 02). `Apply 8mm Web Gusset` would require a gusset/feature primitive that does not exist. |
| 36 | Composer: `Provider: OpenAI / gpt-4o-mini` select | composer | PRESENT | `ChatPanel.tsx:356-386` | Real provider select bound to `server.providers`, disabled when unconfigured. |
| 37 | Composer: `Consent to secure remote LLM inference` checkbox | composer | PRESENT | `ChatPanel.tsx:388-407` | Real consent gate for `openai`/`openrouter`/`codex`; sending is blocked until ticked. |
| 38 | Composer: `Shift+Enter for newline` hint | composer | PRESENT | `ChatPanel.tsx:216-222` | Enter sends, Shift+Enter newlines. |
| 39 | Composer: `SEND ▷` button | composer | PRESENT | `ChatPanel.tsx:433-435` | `[Draft Part Geometry ↵]`, disabled unless `canSend`. |
| 40 | Right card `PARAMETRIC EXTENTS` + `B-Rep Box` badge | right rail | PARTIAL | `Inspector.tsx:111-119`; `types/studio.ts:77-95` | The parameter card exists in Options card 01 with real fields; the badge says `Server verified` / `Local preview` / `No mesh yet` instead of the mock `B-Rep Box`. |
| 41 | Width / Height / Depth sliders + numeric inputs | right rail | PRESENT | `Inspector.tsx:47-87` (`NumberField` + vernier `type="range"`), `:209-219` | Real inputs with backend limit validation (`0.1–2000 mm`, `types/studio.ts:55-56`) and a local error message. |
| 42 | `Mesh Subdivisions: 1` | right rail | PRESENT | `Inspector.tsx:217`; `types/studio.ts:81` | Real (`subdivisions`, 1–64). |
| 43 | `Cylinder Parameters (R, H)` / `Sphere Parameters (Radius)` collapsibles | right rail | PARTIAL | `Inspector.tsx:221-238` | The cylinder and sphere parameter sets exist with all real fields, but they are separate panels, not disclosure widgets. |
| 44 | `NOMINAL CALCULATIONS` card, `B-Rep Exact` badge, Volume / Surface Area / Bounding Envelope | right rail | PARTIAL | `Inspector.tsx:184-202`; `studio/src/viewer/localPrimitives.ts` `computeGeometryEstimate` | All three real numbers are shown. The badge reads `OpenCascade estimate (nominal)` — the build refuses the `B-Rep Exact` claim and adds a disclaimer. Correct behaviour, different wording. |
| 45 | Disclaimer `Estimated via analytical closed-form OpenCASCADE B-Rep solver kernel. Not certified structural compliance.` | right rail | PARTIAL | `Inspector.tsx:195-199` | The build's disclaimer is stronger and honest ("Closed-form nominal geometry… Not a certified FEA solver result"). |
| 46 | `NEUTRAL CAD HANDOFF` card + `ISO 10303` badge | right rail | PARTIAL | `ExportPanel.tsx:133-195`; `OptionsDrawer.tsx:64-69` (`ISO 10303 READY` from `server.stepAvailable`) | Real card, real capability note. |
| 47 | `Target Tool Suite: FreeCAD (neutral file)` select | right rail | PRESENT | `ExportPanel.tsx:143-153` | Real target select built from the CAD integration registry, with per-status labels. |
| 48 | `Protocol: STEP (.step / ISO 10303-21)` | right rail | PRESENT | `ExportPanel.tsx:154-167` | Real format select filtered by the target's real `capabilities`. |
| 49 | `Native socket automation is unmounted …` note | right rail | PRESENT | `ExportPanel.tsx:169-175` | Real note driven by `native_api_unavailable`. |
| 50 | `Download STEP (4,821 B)` | right rail | PARTIAL | `ExportPanel.tsx:181-193` | Real download with the **real** byte count after the fact; before the download the label is `Download STEP` (no pre-computed size). |
| 51 | `Copy CLI Import Command` | right rail | MISSING-BUILDABLE | `ExportPanel.tsx:225-235` | A clipboard copy of a FreeCAD `Import` command is derivable from the real filename + target id already in the panel; no backend change needed. |
| 52 | `CHECKSUM sha256:d84fe0...91c0` | right rail | OMITTED-FAKE | — | The export response carries no checksum (`api/client.ts:287-293`; headers read are `content-disposition` and `x-studio-handoff`). |
| 53 | `RAM Working Set: 142.8 MB` | right rail | MISSING-NEEDS-BACKEND | — | No process-memory endpoint exists; a browser tab also cannot read the backend's RSS honestly. |
| 54 | `GPU Render Core: Metal / Vulkan Active` | right rail | MISSING-NEEDS-BACKEND | — | No GPU/renderer-report endpoint; the renderer is a headless-browser WebGL context, not the backend's. |
| 55 | Bottom status bar: `Connected to http://127.0.0.1:8765 (desktop shell) │ Socket Latency: 1.4ms │ Backend v0.1.0 │ step available │ memory only mode` | bottom | PARTIAL | `JournalPanel.tsx:170-177`; `OptionsDrawer.tsx:54-69, 299-309`; `store.ts:76-81` | Connection origin, backend version and STEP availability are real and shown. `Socket Latency` has no measurement, and `memory only mode` has no backend flag (`server.health` exposes `offline`, `step_available`, `integrations`, `providers` — `src/kala_agent/studio/server.py:179-200`). |

**Screen 2 counts:** PRESENT 12 · PARTIAL 26 · MISSING-BUILDABLE 8 · MISSING-NEEDS-BACKEND 5 · OMITTED-FAKE 3 · N/A 1 · **total 55 rows**

---

# Screen 3 — `kala_studio_master_engineering_design_workbench_night_slate`

Structurally identical to screen 2 (verified: 198 text nodes each, diff = 3 lines). It is the
**palette** reference for the night theme. The night palette *is* applied:
`studio/src/styles/tokens.css:57-81` maps the night-slate values from the Stitch
`tailwind.config` block onto the token set (`--sheet #0a0a0b`, `--card #161619`,
`--secondary/prussian #9ecbfa`, `--on-tertiary-container #dd5a3e` redline, `--outline #777770`,
`--error #ba1a1a`, `--surface-container #161619`, …), and `AppShell.tsx:79` switches the canvas
background to `dark` at night.

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---|---|---|---|---|
| 1 | Top app bar (identity, LIVE, mode, version, theme toggle, account) | header | MISSING-BUILDABLE | as screen 2 rows 1-5 | WAVE conflict (`WAVE.md:27`): the vellum board has no top bar. The night theme exposes the same real facts in the chat stamp / Options drawer. |
| 2 | Theme toggle shows `NIGHT •` active instead of `PAPER •` | header | PARTIAL | `OptionsDrawer.tsx:249-269`; `store.ts` `theme/set` `:142`; `AppShell.tsx:98-101` | The Vellum/Slate switch exists and persists only the theme name (`localStorage['kala-studio.theme']`, `cells/shell/REPORT.md:236`). It lives in the drawer, not a header. |
| 3 | `Bg: Dark Slate` readout | toolbar | PARTIAL | `AppShell.tsx:76-79`; `store.ts:84` `BackgroundKind` | The build derives the canvas background from the theme rather than exposing a `Bg:` control. |
| 4 | Left module rail (`[CAD]`, `[Analysis]`, `[Providers]`, `INTEGRATIONS`) | left rail | PARTIAL | `IntegrationsPanel.tsx:19-46`; `ProviderPanel`; `OptionsDrawer.tsx:199-211` | Real registries, rendered inside the Options drawer. WAVE conflict (the 3-panel shell was not adopted). |
| 5 | `KEYTAR ENCLAVE` origin card | bottom-left | PARTIAL | `OptionsDrawer.tsx:289-296` | Real origin/mode; the "Keytar" name is a mock. |
| 6 | Right-rail `WORKBENCH / EXPORT / ASSISTANT` tabs | right rail | PARTIAL | `OptionsDrawer.tsx:35-40` | Replaced by the 4 drafted Options cards. WAVE conflict. |
| 7 | Toolbar controls (AGENT/3D FOCUS, SHAPE, Grid/Axes/Dims/Section Cut, Mat, Bg, Reset) | toolbar | PARTIAL | `BottomBar.tsx:56-86`; `Inspector.tsx:149-173`; `store.ts:83-84` | Grid/Axes/Dims/Section/Reset/Shape exist. 3D FOCUS, Mat and Bg controls do not. |
| 8 | `2D ORTHOGRAPHIC ↔ 3D ISOMETRIC ORBIT` slider | viewport | MISSING-BUILDABLE | `SceneCanvas.tsx` (24° FOV pencil camera) | No camera-mode control. |
| 9 | `Cut A–A: 50% Section Plane` badge | viewport | PARTIAL | `draftingGeometry.ts:83,96,110,124` | Fixed mid-plane, no offset. |
| 10 | `SERVER VERIFIED MESH / B-Rep v7.8` badge | viewport | PARTIAL | `SceneCanvas.tsx:191-197`; `previewController.ts:139-142` | Real source line; engine string in the journal. |
| 11 | Drafting frame ticks `1`–`8` / `A`–`C` | viewport | PARTIAL | `DraftingSheet.tsx:71-121` | A–F / 1–8 present (two extra rows). |
| 12 | Wireframe part + dimension leaders | viewport | PARTIAL | `SceneCanvas.tsx:652-712` | Real W/H/D; no wall/fillet leaders. |
| 13 | Axis gizmo (`ISO 30° / WGS84 0-beta`) | viewport | OMITTED-FAKE | — | Mock coordinate-system claim. |
| 14 | `SLIDE-RULE COMPANION` overlay | floating card | MISSING-NEEDS-BACKEND | `types/studio.ts:77-95` | No wall/fillet/material/safety-factor data. |
| 15 | `Quick-Spool STEP to FreeCAD` | floating card | PARTIAL | `ExportPanel.tsx:98-131` | Real neutral-file handoff exists; no live spool. |
| 16 | Floating tool palette (dims, Reset Cam, Section Cut, Measure, Wireframe) | floating card | PARTIAL | `DraftingSheet.tsx:188-238`; `BottomBar.tsx:56-97` | Measure and Wireframe missing. |
| 17 | Status strip `[BOX] Dimensions │ Volume │ ISO 10303 READY │ Engine: OpenCASCADE 7.8` | under viewport | PARTIAL | as screen 2 row 29 | Real facts exist across the bottom bar, inspector, journal and Options card 02. |
| 18 | Shortcuts row (4 chips) | under status strip | PRESENT | `ChatPanel.tsx:52-77, 271-287` | Present. |
| 19 | Chat user card + UTC time | chat | PARTIAL | `ChatPanel.tsx:322-335` | Fixed `You` author label. |
| 20 | Chat agent card + engine/model badge + duration | chat | PARTIAL | `ChatPanel.tsx:322-335`; `types/studio.ts:202-207` | Badge and duration not rendered; `ChatResponse` carries no engine/duration. |
| 21 | Inline code chip `t = 8.000 mm` | chat | PARTIAL | `ChatPanel.tsx:333` | No inline code rendering. |
| 22 | `STRUCTURAL CONSTRAINT EVALUATION` PASSED block | chat | OMITTED-FAKE | `Inspector.tsx:195-199`; `OptionsDrawer.tsx:231` | Mock FEA; explicitly disclaimed in the build. |
| 23 | `Apply 8mm Web Gusset` / `Export to .STEP` buttons | chat | PARTIAL | `ExportPanel.tsx:181-188` | Export exists; gusset has no data. |
| 24 | Composer provider select / consent checkbox / Shift+Enter hint / SEND | composer | PRESENT | `ChatPanel.tsx:356-435` | All present and real. |
| 25 | `PARAMETRIC EXTENTS` card + sliders + `Mesh Subdivisions` + collapsibles | right rail | PARTIAL | `Inspector.tsx:47-87, 111-119, 209-239` | Real inputs; not a right-rail card and not disclosure widgets. |
| 26 | `NOMINAL CALCULATIONS` card + disclaimer | right rail | PARTIAL | `Inspector.tsx:184-202` | Real numbers; `B-Rep Exact` downgraded to `nominal` + disclaimer. |
| 27 | `NEUTRAL CAD HANDOFF` card, target select, protocol, note, `Download STEP` | right rail | PARTIAL | `ExportPanel.tsx:133-195` | Real card. |
| 28 | `Copy CLI Import Command` | right rail | MISSING-BUILDABLE | `ExportPanel.tsx:225-235` | Derivable from real filename/target. |
| 29 | `CHECKSUM sha256:…` | right rail | OMITTED-FAKE | — | No checksum in the export response. |
| 30 | `RAM Working Set` / `GPU Render Core` | right rail | MISSING-NEEDS-BACKEND | — | No telemetry endpoint. |
| 31 | Bottom status bar (origin, latency, version, step available, memory only mode) | bottom | PARTIAL | `JournalPanel.tsx:170-177`; `OptionsDrawer.tsx:54-69` | Origin/version/STEP real; latency and memory-only-mode have no source. |
| 32 | Night palette: `#0A0A0B` surfaces, `#9ecbfa` accent, `#dd5a3e` redline, `#777770` outline | whole screen | PRESENT | `tokens.css:57-81`; `studio/src/styles/drafting.css:21-30`; `journal.css:149-170` | The night-slate palette is applied to the tokens, the grid/tick patterns, the drafting surface and the journal. QA verified the live re-theme (`cells/qa/REPORT.md:158`, shot `08-slate-1440-default.png`). |
| 33 | Night-slate layout as a 3-panel workbench | whole screen | N/A | `WAVE.md:13,18-28`; `cells/qa/REPORT.md:186-188` | The WAVE explicitly adopts **only the palette** of this screen; the 4-zone Vellum board remains the night layout. Reinstating the 3-panel shell would contradict the wave. |

**Screen 3 counts:** PRESENT 3 · PARTIAL 21 · MISSING-BUILDABLE 3 · MISSING-NEEDS-BACKEND 2 · OMITTED-FAKE 3 · N/A 1 · **total 33 rows**

---

# Screen 4 — `kala_agent_drafting_kernel_desktop_1600x1000`

A 1600×1000 "drafting kernel" desktop: left calculation sheet, centre drawing, right budget
dock, bottom archival ledger. `WAVE.md:14` lists it as an **element style reference**
(stamps, title blocks, ledger rows, sliders).

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---|---|---|---|---|
| 1 | Top status band `ISO 5457 // FORMAT: A0 SURROGATE • PROJECTION: THIRD ANGLE` | header strip | OMITTED-FAKE | `DraftingSheet.tsx:290-292` (real `3RD ANGLE`) | "A0 SURROGATE" is a mock format claim. The projection statement is real and is rendered in the title block. |
| 2 | Top coordinate ruler `1`–`8` | header strip | PRESENT | `DraftingSheet.tsx:86-103` | Present on the sheet. |
| 3 | `CALIPER: ±0.012mm` / `SCALE: 1:1 METRIC` | header strip | PARTIAL | `AppShell.tsx:86-90` (`scale: 'FIT'`) | The scale is honestly `FIT`, not `1:1`. The ±0.012 mm caliper tolerance is a mock instrument tolerance. |
| 4 | Left vertical ruler `A`–`F` | left edge | PRESENT | `DraftingSheet.tsx:104-121` | Present. |
| 5 | `A` logo tile + `KALA // DRAFTING KERNEL v0.1` brand | top-left | PARTIAL | `ChatPanel.tsx:232-234`; `OptionsDrawer.tsx:148` | Brand exists; there is no logo tile and no separate app identity block (the wave removed the header). |
| 6 | Sheet selector `SHEET 01: mechanical-reference (rev-r0)` | header bar | PARTIAL | `DraftingSheet.tsx:156-162` (`SHEET 01` tab) | One static sheet label; no selector and no rev id. |
| 7 | Workspace tabs `01. MECHANICAL: Bracket Surrogate v0.1.0 (ACTIVE)` / `02. AEROSPACE: Tapered Wing Planform` / `03. ARCHITECTURE: Storey Envelope` | header bar | MISSING-BUILDABLE | `store.ts:94-117` (no sheet model yet) | A multi-workspace switch is a store extension, not a backend one. The three "domains" are mock part names. |
| 8 | `LOOPBACK: 127.0.0.1:8000` / `AIR-GAPPED: LocalModelClient` / `EGRESS: SEALED` / `SANDBOX: PORTABLE` | header right | PARTIAL | `OptionsDrawer.tsx:289-296`; `ChatPanel.tsx:388-419` | Real origin and mode are shown; the "EGRESS: SEALED" / "SANDBOX: PORTABLE" / "LocalModelClient" claims are mock security postures. |
| 9 | Red banner `! NON-AUTHORITATIVE FINDING — SURROGATE REFERENCE ARITHMETIC ONLY (NOT CERTIFIED CAD/CAE)` | full-width banner | PARTIAL | `OptionsDrawer.tsx:231` (`NO CERTIFIED FEA RESULT`); `Inspector.tsx:195-199` | The build carries the same *meaning* as a footer stamp and an inspector disclaimer instead of a banner. |
| 10 | Left rail `CALCULATION SHEET // DIN 4000` + `SEQ: RUN-01` | left rail | N/A | — | "DIN 4000" is a mock standard citation; the build's left rail is the agent chat. |
| 11 | `EVALUATOR SEQUENCE` + `3/4 SATISFIED` counter | left rail | MISSING-BUILDABLE | `types/studio.ts:30` (journal kinds); `store.ts:144` | A real per-step evaluator ledger could be built on the existing journal kinds, but no evaluator concept exists in the backend. |
| 12 | 4 evaluator rows: `✓ 1. geometry-check PASS GeometryEvaluator sha256:8f3a..c1` … `! 4. constraint-eval PENDING AWAITING PRE_COMMIT STAMP` | left rail | OMITTED-FAKE | — | The evaluator names and digests are mock; the `sha256` values are the same placeholders as the master workspace. The build's equivalent is the real journal ledger (`JournalPanel.tsx:141-154`). |
| 13 | `TYPED PARAMETER MATRIX [VERNIER MODE]` header | left rail | PRESENT | `Inspector.tsx:111-119`; `chat.css` | The inspector's parameter matrix with vernier sliders is the same pattern. |
| 14 | `Bracket Width (W) 120.00 mm` slider | left rail | PRESENT | `Inspector.tsx:214` (`size_x`, range 0.1-2000) | Real input + vernier slider, backend-bounded. |
| 15 | `Flange Thickness (t) 16.50 mm` slider | left rail | MISSING-NEEDS-BACKEND | `types/studio.ts:77-95` | No wall/flange-thickness field. |
| 16 | `Bore Hole Dia (Ø) 22.00 mm` slider | left rail | MISSING-NEEDS-BACKEND | as row 15 | No bore field. |
| 17 | `Applied Force (Fz) 4,500.00 N` slider | left rail | MISSING-NEEDS-BACKEND | as row 15 | No load field; the backend runs no FEA. |
| 18 | `Yield Constant (σ_y) 275.0 MPa` slider | left rail | MISSING-NEEDS-BACKEND | as row 15 | No material property in the contract. |
| 19 | Footnote `* footnote: illustrative-constant-not-certified` | left rail | PARTIAL | `Inspector.tsx:195-199` | The build's disclaimer is equivalent and stronger. |
| 20 | Centre drawing `ELEVATION / SECTION A-A` with `W = 120.00 mm`, `t = 16.50`, `⌀20.00 BORE (THRU)` | drawing | PARTIAL | `DraftingSheet.tsx:255-271`; `AuxView.tsx:143-304` | Real W/H/D callouts + a real Section A–A; bore and thickness have no data. |
| 21 | 3D view with `Fz = 4,500 N [DOWN]` force arrow | drawing | MISSING-NEEDS-BACKEND | as row 17 | No load/force concept. |
| 22 | `SURROGATE GEOMETRIC STATE: area: 1,842.5 mm² │ vol: 72,480 mm³ │ σ_bend: 84.2 MPa` strip | drawing | PARTIAL | `BottomBar.tsx:88-97`; `Inspector.tsx:184-202` | Real area and volume are shown; `σ_bend` needs an FEA backend. |
| 23 | Bridge card `kala-agent-freecad v1.1.3 Bridge ONLINE`, `cad.freecad-geometry: 1.0.0 READY`, `STAMP EXPORTS: .STEP .IGES .FCStd`, `CLASS: DESIGN_BINARY` | drawing top-right | PARTIAL | `src/kala_agent/studio/integrations/registry.py:122-131`; `IntegrationsPanel.tsx:19-46`; `BottomBar.tsx:49-52` | The real FreeCAD registry row (`available_via_neutral_file`, capabilities `step, stl`) is rendered in Options card 02 and as a bottom-bar status pill. `v1.1.3`, `ONLINE`, `.IGES`, `.FCStd` and `DESIGN_BINARY` are not real. |
| 24 | `cad.freecad-fem: 1.0.0 STATUS: SOLVER_UNAVAILABLE (CalculiX ccx not present on host machine)` | drawing top-right | MISSING-NEEDS-BACKEND | `registry.py:152-212` | The registry has analysis rows, but no FEM capability is probed, so "SOLVER_UNAVAILABLE" cannot be reported truthfully. |
| 25 | `DIN 6771 // TITLE BLOCK • PRE_COMMIT CHECKPOINT` + `WAITING_FOR_APPROVAL` stamp | title block header | PARTIAL | `DraftingSheet.tsx:273-310`; `JournalPanel.tsx:123-132` | A real drafted title block exists; a checkpoint/waiting-approval state does not. |
| 26 | Title-block fields: `CHECKPOINT ID`, `SPECIFICATION`, `LOCK STATE: GATEWAY_HOLD`, `MATERIAL: Ti-6Al-4V GRADE 5` | title block | MISSING-NEEDS-BACKEND | `types/studio.ts:77-95`; `src/kala_agent/studio/schemas.py` | No checkpoint, lock state, or material property. |
| 27 | `STATE DIGEST sha256:7a91e3...44b8 [VERIFIED MERKLE ROOT]` | title block | OMITTED-FAKE | — | Placeholder digest. The build's `JournalEntry.digest` is only filled from real export byte counts. |
| 28 | `ACTION DIGEST sha256:3c10f8...91d2 [DETERMINISTIC EVAL]` | title block | OMITTED-FAKE | — | Placeholder digest. |
| 29 | `Sign-off Authority: LEAD_ENGINEER_4B // CALIPER_HSM` input | title block | MISSING-NEEDS-BACKEND | — | No sign-off / approval workflow exists. |
| 30 | `STAMP & COMMIT REVISION` / `REJECT & STRIKE` buttons | title block | MISSING-NEEDS-BACKEND | — | No revision-commit or approval API. |
| 31 | `RESOURCE REGISTRY // RUN_BUDGET` + `HARD_LIMITS` + `7-DIMENSION SLIDE-RULES` | right dock | MISSING-NEEDS-BACKEND | — | No budget/run-accounting backend. |
| 32 | 7 budget rows (Model Calls 0/32, Tool Calls 1/64, Context Tokens 0/100k, Artifact Bytes, Wall Duration, Run Cost, Replanning Retries) | right dock | MISSING-NEEDS-BACKEND | — | Same; no run-budget accounting exists. |
| 33 | `SECURITY CAPABILITY SEALS // SANDBOX-ENFORCED` + 4 capability rows (ALLOW/ALLOW/ALLOW/DENIED) | right dock | MISSING-NEEDS-BACKEND | — | No capability-seal service; the studio's real posture is documented as text in Options card 04 (`OptionsDrawer.tsx:221-224`). |
| 34 | `HARDWARE ATTESTATION: YubiKey 5 FIPS` / `SLOT 9A: 9B04 F621 82CD 11A9 E355` | right dock | OMITTED-FAKE | — | Mock HSM attestation. `WAVE.md:36` lists FIPS as a placeholder that must not appear. |
| 35 | `ARCHIVAL EVENT JOURNAL & CAS LEDGER // SQLITE v2 IMMUTABLE HASH-CHAIN` + `STATUS: UNBROKEN TOPOLOGICAL AUDIT` + `PAGE: 01 OF 01` + `ENCRYPTION: HARD-SECURED` | bottom ledger | PARTIAL | `JournalPanel.tsx:121-179`; `store.ts:144-145` (`journal/append`, `JOURNAL_CAP = 500`) | A real, bounded, mono ledger exists with per-kind tags. It is an in-memory event log, not an immutable SQLite CAS chain — the build therefore prints `SESSION LOG`, not `HASH-CHAIN OK`. |
| 36 | Ledger table header `SEQ │ TIMESTAMP (UTC) │ EVENT TYPE │ PAYLOAD & MUTATION DESCRIPTOR │ SHA-256 CAS DIGEST │ CHAIN STATE` | bottom ledger | PARTIAL | `JournalPanel.tsx:141-153` | The build renders tag / time / message / optional digest in a single column, not a 6-column table. |
| 37 | Ledger rows `#20 … #01` with chain states `PENDING / LINKED / ROOT` | bottom ledger | PARTIAL | `JournalPanel.tsx:141-154` | Real rows with real timestamps; there is no sequence number or chain-state column. |
| 38 | Bottom-left `KALA PARAMETRICS (C) 2025 // AIRFRAME DIVISION` | footer | OMITTED-FAKE | — | Mock company/division string. |
| 39 | Bottom-right `DIN 6771 COMPLIANT` | footer | OMITTED-FAKE | — | Mock compliance claim. |
| 40 | Corner `+` drafting marks | drawing corners | PRESENT | `DraftingSheet.tsx:150-153`; `drafting.css:142-155` | Registration crosses present. |

**Screen 4 counts:** PRESENT 5 · PARTIAL 13 · MISSING-BUILDABLE 2 · MISSING-NEEDS-BACKEND 12 · OMITTED-FAKE 7 · N/A 1 · **total 40 rows**

---

# Screen 5 — `kala_agent_floating_overlay_system_3_states`

Three stacked overlay states over a drafting canvas. `WAVE.md:14` lists it as an element style
reference (overlays, seals, stamps).

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---|---|---|---|---|
| 1 | Left rail `KALA // AGENT` + `v4.18` chip | left rail | PARTIAL | `ChatPanel.tsx:232-234`; `OptionsDrawer.tsx:148` | Brand present; `v4.18` is a mock version and there is no separate agent-version source. |
| 2 | `DIN ISO 5457 // ENG` + `A0_CANVAS` | left rail | OMITTED-FAKE | — | Mock standard + mock sheet format. The build's projection statement (`3RD ANGLE`) is real. |
| 3 | Left rail nav: `Viewport Canvas 01`, `Parametric Specs 02`, `GD&T Verifications 03`, `Revision Ledger 04`, `STEP / DXF Output 05` | left rail | PARTIAL | `OptionsDrawer.tsx:35-40` (Inspector / Export / Providers / Connection) | Four real destinations exist. `GD&T Verifications` and `Revision Ledger` have no data; `STEP / DXF` is real for STEP only (the backend exports `stl`/`step`, `types/studio.ts:6`). |
| 4 | `CALIPER STATE / STABLE / SCALE RATIO 1:25.4 METRIC / HASH e8a2f4c9` | left rail footer | OMITTED-FAKE | — | `STABLE`, the 1:25.4 ratio and the hash are placeholders. The build's honest equivalents are `FIT` scale (`AppShell.tsx:88`) and `SESSION LOG` (`JournalPanel.tsx:225`). |
| 5 | Top bar `SHEET: AER-BRK-7702-D │ ZONE [C3-E6]` | top bar | PARTIAL | `DraftingSheet.tsx:156-162`, `:273-310` | A real sheet id (`SHEET 01`) and part id are in the title block; no zone cursor readout. |
| 6 | Top bar `COORDINATES: X: 482.15 mm / Y: 104.88 mm` | top bar | MISSING-BUILDABLE | `SceneCanvas.tsx` camera + `draftingGeometry.ts`; `types/studio.ts:69-72` (`BBox`) | Real mesh bounds exist; a live cursor-to-sheet coordinate readout would need the sheet transform the viewport does not currently expose. |
| 7 | Top bar `2D DRAFT / 3D SOLID` segmented toggle | top bar | PRESENT | `BottomBar.tsx:57-72`; `DraftingSheet.tsx:163-185` | Real 2D/3D switch (two render modes rather than two camera modes). |
| 8 | Top bar account / person icon | top bar | N/A | — | No account concept in a local single-user tool. |
| 9 | Canvas ruler `0 … 150 mm` with `DIN 862 CALIPER` label | canvas | PARTIAL | `SceneCanvas.tsx:343` (`GridHelper`, 1000 mm / 50 divisions); `drafting.css:40` | A real metric grid is drawn; there is no on-canvas ruler scale in millimetres and no caliper instrument. |
| 10 | Canvas drafting border frame + grid | canvas | PRESENT | `DraftingSheet.tsx:85-154`; `drafting.css:57-155` | Present. |
| 11 | STATE 01 floating drafting rule: `[01. MECHANICAL] 127.0.0.1:4099` chip, prompt field, `PROMPT` chip, `EVALUATE` button, `↵`, expand icon, `⌘E` | overlay | PARTIAL | `ChatPanel.tsx:356-435`; `JournalPanel.tsx:170-173` | The prompt input, Enter-to-send and the real loopback origin all exist, but as the persistent left-zone composer, not as a floating in-canvas rule. `⌘E` / `Alt+Space` shortcuts are not bound. |
| 12 | STATE 02 expanded vellum sketchpad HUD (sheet/head line, `Live Topology & Stress Surrogate` italic title, `NON-AUTHORITATIVE // SURROGATE ARITHMETIC ONLY` stamp, `PROJECTION: ISO 30°`, `Fz = 3,850 N`, iso sketch, `GUSSET_T: ACTIVE`, `ISO 1101 TOL: ±0.02`) | overlay | PARTIAL | `OptionsDrawer.tsx:130-236`; `DraftingSheet.tsx:273-310`; `Inspector.tsx:195-199` | The Options drawer is the real expanded drafted card; `3RD ANGLE` is real, `ISO 30°`, `Fz`, `GUSSET_T` and `ISO 1101 TOL` are not. |
| 13 | STATE 02 parameter list `01. WIDTH (w) 120.00 mm`, `02. THICKNESS (t) 12.50 mm`, `03. BORE DIA (Ø) 22.00 mm`, `04. DESIGN LOAD (F) 3,850 N` | overlay | PARTIAL | `Inspector.tsx:209-239` | Width/Height/Depth (and cylinder/sphere) are real inputs; thickness, bore and load have no field. |
| 14 | STATE 02 result `σ_bend: 142.4 MPa` / `Safety Factor: 2.14` | overlay | OMITTED-FAKE | `Inspector.tsx:195-199`; `OptionsDrawer.tsx:231` | Mock FEA output; the build states it computes no stress/deflection/tolerance. |
| 15 | STATE 02 bridge row `BRIDGE: kala-agent-freecad .STEP .FCStd` + `FEM: solver_unavailable (CalculiX ccx absent)` | overlay | PARTIAL | `registry.py:122-131`; `IntegrationsPanel.tsx:19-46` | The FreeCAD registry row and its real `step, stl` capabilities are rendered. `.FCStd` and the CalculiX message are not real. |
| 16 | STATE 02 `STATE_HASH: sha256:7a91e3...44b8 [MATCH]` / `ACT_HASH: sha256:3c10f8...91d2 [MATCH]` | overlay | OMITTED-FAKE | — | Placeholder digests; no backend hash report exists. |
| 17 | STATE 02 `STAMP APPROVAL (PRE_COMMIT)` button + `Open Full Drafting Board →` | overlay | MISSING-NEEDS-BACKEND | `ExportPanel.tsx:181-193`; `OptionsDrawer.tsx:130-236` | A real primary action exists in the drawer; a pre-commit stamp workflow does not. |
| 18 | STATE 03 modal `CRIT-A1 // AUTHORIZATION REQUIRED` + `GATE_SEC: 04` | overlay | MISSING-NEEDS-BACKEND | `ChatPanel.tsx:388-407` | A real remote-egress consent gate exists for chat; a general "tool class change" approval with a security gate id does not. |
| 19 | STATE 03 title `First-Use Authorization: Tool Class Change` + body copy | overlay | MISSING-NEEDS-BACKEND | as row 18 | Same. |
| 20 | STATE 03 fact rows `TARGET SUBSYSTEM`, `ISOLATION JAIL (chroot, net_iso)`, `CAPABILITIES artifact.write, tool.execute`, `ACTION DIGEST`, `TIMESTAMP / HSM` | overlay | MISSING-NEEDS-BACKEND | `OptionsDrawer.tsx:221-224`; `src/kala_agent/studio/middleware.py` | The isolation-jail claims are unverified for this deployment; the action digest and HSM timestamp have no source. |
| 21 | STATE 03 `AFFIX APPROVAL SEAL` / `REFUSE & ABORT EXECUTION` buttons | overlay | MISSING-NEEDS-BACKEND | as row 18 | No approval service. |
| 22 | STATE 03 footer `SIGNATURE: RSA-PSS-4096` / `NONCE: 0xFD89_2210` | overlay | OMITTED-FAKE | — | Mock signature/nonce. |
| 23 | Bottom overlay-coupling card `KALA OVERLAY COUPLING // AGENT_CAD` + description + `STATUS: SYNCHRONIZED` / `LATENCY: 4.2ms` / `BUFFER: CLEAN` | overlay | PARTIAL | `OptionsDrawer.tsx:191-198`; `JournalPanel.tsx:170-177`; `BottomBar.tsx:88-97` | Real status/latency-free facts are shown elsewhere (journal origin + version, bottom-bar source badge). `LATENCY: 4.2ms` has no measurement. |
| 24 | Overlay z-order / float above the canvas | overlay | MISSING-BUILDABLE | `studio/src/styles/layout.css`; `OptionsDrawer.tsx:130-143` (`options-backdrop`, `position`) | The drawer is the only floating layer today; a floating in-canvas rule would be a new layer in `layout.css` / `drafting.css`. |

**Screen 5 counts:** PRESENT 2 · PARTIAL 9 · MISSING-BUILDABLE 2 · MISSING-NEEDS-BACKEND 5 · OMITTED-FAKE 5 · N/A 1 · **total 24 rows**

---

# Totals by status

| Status | S1 vellum board | S2 day workbench | S3 night slate | S4 drafting kernel | S5 floating overlay | **Total** |
|---|---:|---:|---:|---:|---:|---:|
| PRESENT | 14 | 12 | 3 | 5 | 2 | **36** |
| PARTIAL | 21 | 26 | 21 | 13 | 9 | **90** |
| MISSING-BUILDABLE | 2 | 8 | 3 | 2 | 2 | **17** |
| MISSING-NEEDS-BACKEND | 3 | 5 | 2 | 12 | 5 | **27** |
| OMITTED-FAKE | 3 | 3 | 3 | 7 | 5 | **21** |
| N/A | 2 | 1 | 1 | 1 | 1 | **6** |
| **Rows** | **45** | **55** | **33** | **40** | **24** | **197** |

Weighted view: **126 / 197 rows (63.9 %) are built in some form** (PRESENT + PARTIAL), 17 are
buildable today from existing data, 27 need backend capability, 21 are correctly-omitted
placeholder data, 6 are pure mock decoration.

---

# Top gaps (ranked)

Ranked by look-and-usability impact on the **primary target** (the Vellum board) first, then by
impact on the other four screens. "Needs backend" rows are listed separately below and are
**not** ranked here.

| Rank | Gap | Why it matters | Concrete file that would change |
|---:|---|---|---|
| 1 | **No multi-sheet tab bar** — the Stitch shows `Sheet 1: L-Bracket` (active, `✕`), `Sheet 2: Stepped Shaft`, `Sheet 3: Flange Housing` and a `+` button; the build renders one static `SHEET 01` tab. | This is the most visible structural difference in the centre column header and it is the entry point to "multiple drawings". The `+` affordance is a dead end today. | `studio/src/components/drafting/DraftingSheet.tsx:156-162` (tab markup) + `studio/src/state/store.ts` (new `sheets` state, `sheet/select`, `sheet/add`, `sheet/close`) + `studio/src/styles/drafting.css:158-199`. |
| 2 | **Bottom-bar centre readout lost `SCALE` and has no verification stamp** — Stitch: `SCALE: 1:1 METRIC │ ENV │ VOL │ [✓ B-Rep Verified]`; build: `ENV │ VOL │ AREA │ [server]`. | The scale statement is the single most-looked-at fact on a drafting sheet. `FIT` is already available (`AppShell.tsx:88`) — showing `SCALE FIT` in the readout is a one-line honest win. | `studio/src/components/bottombar/BottomBar.tsx:88-97` (add a `Readout label="SCALE"`) + `studio/src/components/shell/AppShell.tsx:81-96`. |
| 3 | **The sheet tool palette is missing 4 of the 5 Stitch tools** — Stitch has a 2D/3D toggle, Select, Mark/Redline, Measure/Caliper and Eraser; the build has Reset / Dims / Section A–A / Grid. | The palette is a focal point of the centre column and the Stitch reads as a drawing tool, the build as a viewport toolbar. Select/Mark/Eraser have no implementation, but the *measure* and *redline* concepts are partly real (`showDimensions`, `showSection`). | `studio/src/components/drafting/DraftingSheet.tsx:188-238` (add a redline/measure button) + `studio/src/state/store.ts` (new `showRedlines` flag) + `studio/src/styles/drafting.css:247-301`. |
| 4 | **Journal ledger is a single-column list, not the 6-column CASE ledger** — Stitch rows are `SEQ │ TIMESTAMP │ EVENT TYPE │ PAYLOAD & MUTATION DESCRIPTOR │ SHA-256 CAS DIGEST │ CHAIN STATE`. | The ledger is 65 % of the right column. A sequence number, a distinct descriptor column and a digest column are all buildable from the data the store already holds (`JournalEntry.id`, `.ts`, `.kind`, `.message`, `.digest`). | `studio/src/components/journal/JournalPanel.tsx:141-154` (row markup) + `studio/src/styles/journal.css` (grid columns) + `studio/src/state/store.ts:144-145` (add a `seq` counter to `JournalEntry`). |
| 5 | **Aux panel footer lost the real "PROFILE / PLANE" wording and the redline `CUT: 45° ANSI` moved** — Stitch has it in the header (redline) and a two-part note in the footer; the build has it only in the footer, with different strings. | Small, but the aux panel is the most-read panel after the sheet; a redline cut note in the header was the strongest visual accent of that panel. | `studio/src/components/drafting/AuxView.tsx:104-138` (add a header note element) + `studio/src/styles/drafting.css:453-505`. |
| 6 | **No "jump to Section A–A" affordance from the chat** — the Stitch shows a Prussian check pill `Section A-A updated in aux view`; the build computes the section but offers no way to reach it from the conversation. | The chat and the aux view are the two ends of the same loop; the pill is what closes it. Zero new data needed — `auxView` is already in the store. | `studio/src/components/chat/ChatPanel.tsx:388-419` (add a pill that dispatches `auxView/set` → `'section'`) + `studio/src/styles/chat.css`. |
| 7 | **No `Mat:` / `Bg:` controls** — `MaterialKind` and `BackgroundKind` exist in the store and the renderer implements them, but nothing in the UI writes them. | The Stitch exposes both in the top toolbar; the field is dead state today. | `studio/src/components/bottombar/BottomBar.tsx:56-86` (add two selects) + `studio/src/components/shell/AppShell.tsx:76-79` (stop deriving the background from the theme, or add an override). |
| 8 | **No `Copy CLI Import Command`** — the Stitch has it next to the download button, and the real filename + target id are already in the panel state. | A one-function win that matches a real operator workflow (paste an import command into FreeCAD). | `studio/src/components/export/ExportPanel.tsx:181-195` (add a button + `navigator.clipboard.writeText`). |
| 9 | **Chat card has no engine/model badge and no elapsed time** — the Stitch agent card shows `OpenCASCADE v7.8 · gpt-4o-mini · Deterministic` and `(2.8s)`. The real provider/model are known to the request; the duration is measurable client-side. | It is the metadata an operator needs to trust an answer. | `studio/src/components/chat/ChatPanel.tsx:322-335` + `studio/src/state/store.ts:151-152` (add `model`/`at` to the assistant message). |
| 10 | **The drafted centre lines with `+X (WIDTH)` / `+Y (HEIGHT)` labels are not drawn** — the build has only the 3D `AxesHelper`. | The drafted axis annotation is a signature of the drafting look, and it is also the orientation cue the Stitch uses for the redline dimensions. | `studio/src/viewer/SceneCanvas.tsx:339` (axes build) + `studio/src/components/drafting/DraftingSheet.tsx:240-271` (an SVG overlay layer above the canvas). |

---

# Needs backend

Grouped by the backend capability that would have to exist. Each entry names the Stitch row(s)
that depend on it.

**A. Geometry feature model (holes, fillets, wall thickness, material, loads)**
`src/kala_agent/studio/schemas.py`, `src/kala_agent/studio/geometry/**`, `studio/src/types/studio.ts:77-95`
- Bore / hole diameter + fit class (H7) and surface finish (Ra): S1 rows 30; S2 row 24; S4 rows 16, 20; S5 row 13.
- Fillet radius: S1 row 31; S2 row 24; S4 row 15; S5 row 13.
- Wall / flange thickness: S4 row 15; S5 row 13.
- Applied force / design load: S4 row 17, 21; S5 row 13.
- Material designation (Al 6061, Ti-6Al-4V) and yield constant: S1 row 8; S4 rows 18, 26; S5 row 12.
- Safety factor: S1 row 33 (stitch `safety_f`); S2 row 26; S5 row 14.
→ These need a real feature/parameter model, not just a tolerance block.

**B. Engineering analysis / FEA**
- Stress, deflection, safety factor, constraint evaluation: S2 rows 34, 35; S3 row 22; S4 rows 11, 12, 21, 22, 24; S5 row 14.
- A named analysis solver (ANSYS/Abaqus/COMSOL/Nastran/OptiStruct) that reports availability: S2 row 9; S4 row 24.
→ `src/kala_agent/studio/integrations/registry.py` lists the analysis rows, but no capability is probed.

**C. Tolerance / GD&T model**
- Tolerance stack, fit class, ISO 2768 / ISO 1101 class, GD&T verifications: S1 row 32 (`TOLERANCE` cell), S2 row 34 (the `±0.05` in the constraint block), S3 row 22, S4 row 15, S5 rows 3, 12.

**D. Cryptographic audit ledger (hash chain / merkle root / CAS)**
- A hash-chain verification report the UI can show as `HASH-CHAIN OK` / `BROKEN`: S1 rows 41, 43, 44; S4 rows 12, 27, 28, 35, 36, 37; S5 row 16.
- Currently `hashChainStatus` is hard-wired to `'unknown'` (`studio/src/components/shell/AppShell.tsx:140`) and the prop already accepts `'verified' | 'broken'` (`studio/src/types/studio.ts:42`, `JournalPanel.tsx:221-226`).
- Per-entry digests for previews and chat (not only exports): S1 rows 10, 43.

**E. B-Rep / mesh verification report**
- A backend verdict that justifies a `B-Rep Verified` stamp: S1 rows 36, 45; S2 row 22; S3 row 10.

**F. Approval / sign-off workflow**
- Checkpoint, lock state, sign-off authority, stamp & commit / reject & strike, approval seal, security capability seals, signed nonces: S4 rows 25, 26, 29, 30, 33; S5 rows 17, 18, 19, 20, 21, 22.

**G. Run budget / telemetry**
- Model-call, tool-call, token, byte, wall-clock and cost accounting: S4 rows 31, 32.
- Process memory (RAM) and renderer/GPU reporting: S2 rows 53, 54.
- Socket latency measurement: S2 row 55, S3 row 31, S5 row 23.
- "memory only mode" backend flag: S2 row 55.

**H. Native CAD automation / live bridge**
- A live FreeCAD (or SOLIDWORKS / Fusion 360) socket bridge with a version, a workspace id and a `.IGES`/`.FCStd` exporter: S2 row 27, S3 row 15, S4 row 23, S5 row 15.
- A running FEM solver probe (CalculiX presence): S4 row 24.

**I. Live view / camera model**
- Named cameras, saved views, a "3D focus" framing action and an operator-movable section plane: S2 rows 14, 20, 21; S3 rows 8, 9.

**J. Document / geometry integrity digest**
- A real checksum (sha256) for exported artefacts, to populate the `CHECKSUM` field and the per-log digest chips: S2 row 52; S3 row 29; S1 row 10.

---

# Explicitly not fabricated (audit confirmation)

The following Stitch values were checked in `studio/src/**` and are **absent**, which is the
correct outcome per `WAVE.md:36` (invariant 2, "no fake data"):

`28.16 cm³` · `sha256:8f3a…` · `sha256:7a91e3…44b8` · `sha256:3c10f8…91d2` ·
`sha256:d84fe0…91c0` · `e8a2f4c9` · `FreeCAD 1.1.3` · `v1.1.3 Bridge` · `PID 14208` ·
`Socket Latency: 1.4ms` · `LATENCY: 4.2ms` · `RAM Working Set 142.8 MB` ·
`FIPS` / `YubiKey 5` · `Llama-3` · `HASH-CHAIN OK` (only in the guarded `status === 'verified'`
branch at `studio/src/components/journal/JournalPanel.tsx:222`, unreachable because
`AppShell.tsx:140` passes `'unknown'`) · `B-Rep Verified` · `ISO 2768-m` · `ISO 1101 TOL` ·
`σ_bend` / `σ_y` · `KEYTAR` · `DIN 4000` / `DIN 5457` / `DIN 6771` / `DIN 862` ·
`A0 SURROGATE` · `AER-BRK-7702-D` · `KALA PARAMETRICS (C) 2025` · `WGS84`.

(`ISO 10303` and `3RD ANGLE` **are** present and are honest — `OptionsDrawer.tsx:68` derives
the first from `server.stepAvailable`, and `AppShell.tsx:90` states the second as the drafting
convention.)

A `grep -rn "sha256" studio/src` returns **zero** matches; the only digest in the codebase is
`JournalEntry.digest`, filled from a real export byte count
(`studio/src/components/export/ExportPanel.tsx:119-123`).
