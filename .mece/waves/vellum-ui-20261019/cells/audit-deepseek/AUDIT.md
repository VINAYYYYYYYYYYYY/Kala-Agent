# Element audit — audit-deepseek (read-only)

Wave: Vellum Drafting Board UI, 2026-10-19. Auditor: **audit-deepseek**.
Scope: the five assigned Stitch screens plus the two assigned documents in
`cells/AUDIT-TASK.md` (the Stitch prompt element list and the DESIGN.md token
check), and the Solid Edge reference. I read `../WAVE.md`, the four cell REPORTs, all assigned
`stitch/*/code.html` and `screen.png` files, `qa/shots/*.png`, and the built
`studio/src` tree. I did not edit any other file, run a commit, delegate, or
read credentials. I did not run the app; built-UI evidence is source plus the
P3 screenshots.

## Method and status legend

Elements were enumerated from each `code.html`/`screen.png` (panels, headings,
buttons, tabs, inputs, sliders, stamps, tables, icons, drafting furniture and
interaction states) and then traced into `studio/src`. A screen's Stitch value
is checked against the honesty rules: the WAVE invariant 2
(“no fake data”) is applied, so a placeholder value that has no real source is
recorded as `OMITTED-FAKE` even when the visual slot exists.

- **PRESENT** — built and functional; evidence cited.
- **PARTIAL** — the element exists but differs (state, placement, static vs
  live, or density). The difference is stated.
- **MISSING-BUILDABLE** — not built, but the data/function already exists in
  the frontend store, the Studio API, or `src/kala_agent/**`.
- **MISSING-NEEDS-BACKEND** — not built and no honest Studio data exists yet;
  the needed capability is named.
- **OMITTED-FAKE** — intentionally omitted because the Stitch value is
  placeholder data, or the built UI deliberately shows a neutral state instead
  of claiming it.
- **N/A** — mock decoration or a different product surface with no meaning for
  this wave; the one-line reason is in Notes.

Paths: component/file aliases used in the tables resolve as follows. `OPT` =
`studio/src/components/options/OptionsDrawer.tsx`; `INS` =
`studio/src/components/inspector/Inspector.tsx`; `EXP` =
`studio/src/components/export/ExportPanel.tsx`; `PRV` =
`studio/src/components/panels/ProviderPanel.tsx`; `INT` =
`studio/src/components/panels/IntegrationsPanel.tsx`; `SHEET` =
`studio/src/components/drafting/DraftingSheet.tsx`; `AUX` =
`studio/src/components/drafting/AuxView.tsx`; `BAR` =
`studio/src/components/bottombar/BottomBar.tsx`; `JRN` =
`studio/src/components/journal/JournalPanel.tsx`; `CHAT` =
`studio/src/components/chat/ChatPanel.tsx`; `APP` =
`studio/src/components/shell/AppShell.tsx`; `BOOT` =
`studio/src/components/shell/BootGate.tsx`; `CONN` =
`studio/src/components/shell/ConnectionForm.tsx`; `SCENE` =
`studio/src/viewer/SceneCanvas.tsx`; `TOK` = `studio/src/styles/tokens.css`;
`BASE` = `studio/src/styles/base.css`; `LAYOUT` =
`studio/src/styles/layout.css`; `CSS` = `studio/src/styles/studio.css`;
`DRAFT` = `studio/src/styles/drafting.css`; `CHATCSS` =
`studio/src/styles/chat.css`; `JRCSS` = `studio/src/styles/journal.css`;
`LOCALPRIM` = `studio/src/viewer/localPrimitives.ts`; `MAIN` = `studio/src/main.tsx`;
`STORE` = `studio/src/state/store.ts`; `TYPES` =
`studio/src/types/studio.ts`; `CLIENT` = `studio/src/api/client.ts`;
`SERVER` = `src/kala_agent/studio/server.py` (repo root);
`JOURNAL_PY` = `src/kala_agent/persistence/journal.py` (repo root).

---

## 1. `kala_studio_options_engineering_drawer_wireframe_sheet`

The Stitch sheet is a full-page A2 overlay with a 2×2 card grid and a footer
title block. The built UI is the modal `OptionsDrawer` mandated by WAVE
(`WAVE.md:28`). Elements are compared to the modal.

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---------|---------------|--------|-------------------|-------|
| 1 | Overlay sheet / backdrop | Full vellum A2 sheet with Y-datum marks and outer registration frame | `PARTIAL` | `OPT:130-143`; `CSS:890-910`; QA `06-vellum-1440-options.png` | Centred modal (max 1180×92vh), not a full-page sheet. WAVE explicitly requests a modal. |
| 2 | Workbench export chrome | `SEC-A // 01`, `REF: 1440x900`, `Y-DATUM 00/90`, “Kala Studio Archival Drafting Sheet” | `N/A` | Stitch `code.html:1-5`; `LAYOUT:11-20` | Outer paper/registration frame is a mock-export convention; the app is the workbench itself. |
| 3 | Drawer title line | “Kala Studio // Options & Engineering Drawer” | `PRESENT` | `OPT:148` | Exact wording preserved. |
| 4 | Drawer reference tag | `[DRAWER_REF: 0x8F3A]` / `[DRAWER: ACTIVE (0x8F3A)]` | `OMITTED-FAKE` | `OPT:144-158` | No fake hex reference is shown. |
| 5 | Drawer subtitle | “Secondary Controls, Kernel Overrides & Handoff Protocols” | `PRESENT` | `OPT:149` | Reworded to include connection recovery; same role. |
| 6 | Pin Calipers action | “Pin Calipers” button in the mock’s browser chrome | `N/A` | Stitch `code.html:19-21` | No calibration/pinning feature exists in the Studio; decorative chrome. |
| 7 | Close / dock to ruler | “✕ Close / Dock to Scale Ruler [Esc]” | `PARTIAL` | `OPT:154-157`; `OPT:87-91`; focus trap `OPT:71-114` | Esc closes and focus returns to the trigger; there is no “dock to the ruler bar” state (modal only). |
| 8 | Standards reference | `DIN EN ISO 5457 // FORMAT-A2 REF` | `MISSING-BUILDABLE` | `OPT:228-232`; `CSS:1189-1200` | The footer only prints scale/projection. A static standards cell can be added without backend data. |
| 9 | 2×2 drafted grid | Four cards arranged 2×2, all on one sheet | `PARTIAL` | `OPT:174-226`; `CSS:1023-1031` | Two columns with internal scroll; four cards are never visible together. Card 01 also stretches to the tall row (QA REPORT §2.3). |
| 10 | Card registration marks | Four `+` marks, one per card corner | `PARTIAL` | `CSS:1044-1053` | Only one `+` (top-left, via `::before`); no bottom/right marks. |
| 11 | Card hard offset shadow | `shadow-[2px_2px_0px_#1c1c18]` on every card | `N/A` | `WAVE.md:32`; `CSS:1-6` | WAVE forbids drop shadows; flat 1px/0.5px drafted borders are the wave rule. |
| 12 | Card title-bar icons | settings / output / vpn_key / sync_alt glyphs | `PARTIAL` | `OPT:238-247` | CardHeader renders index + title + note only; no icon slot. |
| 13 | Card 01 header | “01 // Parametric Inspector & Primitive Controls” | `PRESENT` | `OPT:35-40, 181` | Exact index/title. |
| 14 | CAD-core version | `CAD-CORE: v2.4.9` | `OMITTED-FAKE` | `OPT:181`; QA `06-…options.png` | No fabricated kernel version; the live source badge says `SERVER VERIFIED` / `LOCAL PREVIEW`. |
| 15 | Shape selector | Box / Cylinder / Sphere / Custom Script tabs | `PARTIAL` | `INS:143-173`; `TYPES:5-6` | Three real primitive tabs. “Custom Script” is not in the backend primitive schema (Box/Cylinder/Sphere only) and is not built. |
| 16 | Active shape tab | Inverted black active tab | `PRESENT` | `CSS:482-486`; `INS:158-166` | Active primitive is inverted with `aria-checked`. |
| 17 | Width field | `Width (size_x) // Base Span` + numeric `60.0` + `mm` | `PRESENT` | `INS:214`; `INS:47-73` | Field label is “Width (size_x)” with a `(mm)` suffix and numeric input. |
| 18 | Height field | `Height (size_y) // Elevation` + numeric + `mm` | `PRESENT` | `INS:215` | Same control family; min/max validation. |
| 19 | Depth compact card | `Depth (size_z) 40.0 mm` with a filled progress bar | `PARTIAL` | `INS:216`; `INS:47-86`; `CSS:494-527` | Present as a numeric field + native range slider; not the compact split row/fill bar. |
| 20 | Wall / bore parameter | `t_wall (Bore) 8.0 mm` with red emphasis | `MISSING-NEEDS-BACKEND` | `TYPES:77-97`; `INS:209-240` | The primitive schema has no wall thickness or bore; there is nothing honest to bind it to. |
| 21 | Vernier ruler graphics | Etched tick rails, major/secondary ticks, sliding thumb under Width/Height | `PARTIAL` | `INS:74-83`; `CSS:524-527`; `BASE:74-79` | A native `<input type="range">` is used with accent colour only: no ticks, thumb triangle, or value tooltip. |
| 22 | OpenCascade stamped box | Double-bordered “OPENCASCADE NOMINAL ESTIMATE” panel | `PRESENT` | `INS:184-201`; `CSS:549-555` | 2px double ink border and section heading match the drafting stamp. |
| 23 | Drift stamp | `✓ DRIFT < 0.01 mm NOMINAL` | `PARTIAL` | `INS:248-267`; `INS:274-289` | A real local-vs-server bounds drift warning exists, but only fires above 0.01 mm; there is no affirmative “nominal” stamp (correctly not fabricated). |
| 24 | Volume readout | `28,160 mm³ (28.16 cm³)` | `PRESENT` | `INS:190`; `LOCALPRIM:546-549` | Real closed-form nominal volume, exact units. |
| 25 | Surface-area readout | `9,420 mm²` | `PRESENT` | `INS:191`; `LOCALPRIM:552-555` | Real closed-form area. |
| 26 | Honesty footnote | Italic “Closed-form nominal geometry … Not a certified FEA solver result.” | `PRESENT` | `INS:195-199` | Exact policy in different line wrapping. |
| 27 | Card 02 header / capability stamp | `02 // Neutral-File Export & CAD/CAE Handoff` + `ISO 10303 READY` | `PRESENT` | `OPT:37, 191`; `OPT:64-69` | Stamp reads the live `server.stepAvailable` state (`STEP —` / `ISO 10303 READY` / `STEP UNAVAILABLE`). |
| 28 | Target host dropdown | FreeCAD 1.1 (Connected), SOLIDWORKS 2024, Fusion 360, ANSYS, Abaqus | `PRESENT` | `EXP:143-153`; `INT:19-45`; QA `06-…options.png` | Uses the live CAD registry rows, with honest status suffixes instead of the fixed mock list. |
| 29 | “Connected” target claim | `FreeCAD 1.1 (Connected)` / “Direct Socket / Pipe” | `OMITTED-FAKE` | `EXP:145-152, 207-218`; `INT:8-12` | Registry says `neutral file` / `native API unavailable`; the UI never claims a live socket or PID. |
| 30 | Format radio pills | STEP (AP214) / STL Mesh / Native FCStd | `PARTIAL` | `EXP:30, 154-167` | A `<select>` offers only the two real formats, STL and STEP; no FCStd writer exists. AP214 is not claimed. |
| 31 | Primary stamped export | “Export & Stamp STEP Artefact ↵” with verified icon | `PARTIAL` | `EXP:181-188` | Button is “Download STL/STEP”; it performs the real export but does not claim a stamp/verification. |
| 32 | Receipt / verification seal | `Saved: … 14,820 bytes`; `SHA-256: e41b… [VERIFIED]` | `PARTIAL` | `EXP:189-193`; `EXP:119-123` | Real filename and byte count are shown and journaled. The Studio response carries no SHA-256, so no hash is printed. |
| 33 | Copy CLI Bridge | Button copies a CLI bridge command | `MISSING-NEEDS-BACKEND` | `EXP:133-196`; `SERVER:179-376` | No CLI-bridge/spool endpoint or command data exists; a dead copy button was not added. |
| 34 | Spool Edge Macro | Button spools a Solid Edge macro | `MISSING-NEEDS-BACKEND` | `EXP:133-196` | No macro/spool capability or filesystem endpoint. |
| 35 | Card 03 header / vault tag | `03 // Model Gateway & API Providers` + `[LOCAL VAULT]` | `PRESENT` | `OPT:205`; `OPT:38` | Exact wording. |
| 36 | Key-security note | “API keys … local memory / OS Keytar enclave” | `PARTIAL` | `OPT:206-209` | The note is present but describes the real backend-only settings file, not in-memory Keytar; no Keytar claim. |
| 37 | Provider rows | Local helper, OpenRouter, OpenAI Compatible | `PRESENT` | `PRV:76-167`; QA `06b-vellum-1440-options-providers.png` | Rows come from the live provider registry (local, OpenAI, OpenRouter, Codex). |
| 38 | Provider status stamps | `[ ACTIVE · OFFLINE ]`, `[ CONFIGURED ]`, `[ NOT CONFIGURED ]` | `PARTIAL` | `PRV:92-96` | Built status is `Configured server-side` / `Not configured`; it does not claim ACTIVE or OFFLINE per provider. |
| 39 | Local model name | `Llama-3-Parametric-8B (GGUF q4_k_m)` | `OMITTED-FAKE` | `PRV:98` | No model claim; the real backend note is printed instead. |
| 40 | Masked key input + Save/Clear | Pre-filled `sk-or-v1-••••…`, Save, Clear | `PARTIAL` | `PRV:119-150` | Password input is empty by design (secrets are never echoed), Save key posts to `/settings`, Clear key appears only when configured. Safer than the mock. |
| 41 | OpenAI “Mount” action | `Mount` button beside a masked OpenAI key | `PARTIAL` | `PRV:132-138`; `CLIENT:177-183` | Implemented as `Save key` against the real `POST /api/studio/settings`. |
| 42 | Remote egress allowlist | Segmented `[OFF / ON]` toggle in card 03 | `PARTIAL` | `CHAT:388-407`; `CHAT:150-157` | The real gate is a per-request consent checkbox in the chat composer, not a persisted drawer allowlist. |
| 43 | Enclave / FIPS footer | `ENCLAVE: SEC-CHIP-T2`; `FIPS-140-3 COMPLIANT LEVEL 1` | `OMITTED-FAKE` | `OPT:228-232` | No such hardware/FIPS claim; footer is honest sheet metadata. |
| 44 | Card 04 header / port | `04 // Loopback Boot Gate & Recovery` + green `PORT 8765` | `PRESENT` | `OPT:219`; `OPT:54-69` | Port is parsed from the real connected origin; before connect it reads `NO LOOPBACK`. No fake green dot. |
| 45 | Origin field | Editable `http://127.0.0.1:8765` with status dot | `PARTIAL` | `OPT:290-293`; `CONN:57-70` | Card 04 shows the live origin as a read-only fact. The editable origin + token form lives in the BootGate fallback, per WAVE. |
| 46 | Bearer-token field / visibility | Masked token + eye toggle | `OMITTED-FAKE` | `OPT:311-313`; `CONN:71-83` | Token is never displayed or prefilled; the card states “In memory only (not displayed)”. |
| 47 | Rotate Token | Rotate the per-launch bearer token | `MISSING-NEEDS-BACKEND` | `OPT:271-330`; `SERVER:179-376` | No token-rotation endpoint; adding one is a security-sensitive backend change. |
| 48 | Connect Loopback | Primary “Connect Loopback” button | `PARTIAL` | `CONN:51-90`; `OPT:315-324` | Manual connection exists in the BootGate form; drawer card 04 offers only reconnect/retry. WAVE assigns connect to the gate. |
| 49 | Retry Bridge | Secondary “Retry Bridge” button | `PRESENT` | `OPT:315-324` | “Reconnect / Retry Bridge” dispatches `connection/retry` and journals the action. |
| 50 | In-memory privacy notice | Bold “Notice:” that the token is held in-memory only | `PRESENT` | `OPT:221-224` | Exact policy, restyled. |
| 51 | IPC latency / socket / PID | `IPC LATENCY: 1.4ms`, `SOCKET: ESTABLISHED`, `PID: 14208` | `OMITTED-FAKE` | `OPT:271-330` | No telemetry route; none of these numbers is displayed. A real telemetry endpoint would be needed if wanted. |
| 52 | Drawer title block | Studio ref, sheet number, drawing title, scale/projection, rev/approval | `PARTIAL` | `OPT:228-232`; `CSS:1189-1208` | Footer has studio name, scale/projection, and a “NO CERTIFIED FEA RESULT” stamp; no sheet number, drawing title, revision, or approval cell. |
| 53 | Interaction states | Hover/active tabs, Esc close, focus trap, focus return, empty connection | `PRESENT` | `OPT:71-126`; `CSS:985-1021`; QA REPORT §4 | Focus is trapped; Esc closes; focus returns to the invoking trigger (`.bottom-bar-options` when opened there); empty states read `NO LOOPBACK` / `—`. |

Counts: 53 rows — `PRESENT` 18 · `PARTIAL` 20 · `MISSING-BUILDABLE` 1 · `MISSING-NEEDS-BACKEND` 4 · `OMITTED-FAKE` 7 · `N/A` 3.

---

## 2. `drawing_sheets_title_blocks`

This is a mobile sheet-folio/plotter screen. The product wave has no
multi-sheet folio or plotter; the useful conventions are the title block,
sheet metadata, stamps, and the technical preview. Those are compared to the
built sheet title block and the AuxView. Sheet 02/03 and the plotter are real
features of the mock that the product does not have.

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---------|---------------|--------|-------------------|-------|
| 1 | Fixed app header | `SEC. C4`, `COORD: 39°N / 104°W`, `CALIPER: SYNCED [2H]`, `SCALE 1:2.5` | `N/A` | Stitch `code.html:7`; `WAVE.md:27` | Decorative coordinate chrome; WAVE requires no top header bar. |
| 2 | Brand block | `K`, `KALA PARAMETRICS // Sheets`, `SPEC: 8402-AE // REVISION: D.4` | `N/A` | Stitch `code.html:7`; `CHAT:231-238` | The product is branded as the mechanical agent, not a multi-sheet “Kala Parametrics” folio; the visible app stamp exists. |
| 3 | Inspect Coordinate button | Tune glyph, `aria-label="Inspect Coordinate"` | `N/A` | Stitch `code.html:7` | No coordinate inspector exists or is needed for the single-part workbench. |
| 4 | Profile avatar | Remote `lh3.googleusercontent.com` portrait | `N/A` | Stitch `code.html:7`; `WAVE.md:16` | Single-user local app; remote assets are forbidden. |
| 5 | Sheet-management header | `ISO-5457 // DRAWER 04`, `SPOOLER: READY` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:9-19`; `SERVER:1-25` | No sheet registry or spooler; the Studio server has no sheet routes. |
| 6 | Title / project line | `PROJECT KALA-AE-84 // FLANGE COUPLING`, “Drawing Sheets & Plotter Export” | `N/A` | Stitch `code.html:21-24` | Different screen concept (sheet folio) not in the 4-zone wave; the drafting sheet has its own title block. |
| 7 | Standards + sheet count | `ISO 128-30 / DIN 6771`, `3 SHEETS REGISTERED` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:25-28` | Needs a real sheet record/count API. |
| 8 | Engineering folio + calibrate | `Engineering Folio [SERIES 8402]`, `CALIBRATE MARGINS` | `N/A` | Stitch `code.html:30-43` | No multi-sheet folio or margin calibration feature. |
| 9 | Selected sheet card | `SH-01: Orthographic Projections`, `SELECTED` tag | `PARTIAL` | `SHEET:156-186`; `APP:83` | The sheet tab reads `SHEET 01`; there is no gallery of selectable sheets. |
| 10 | Miniature blueprint preview | Grid, flange circles, PCD bolt holes, centerlines, hatched front section, `56.00`, `Ø 120.0 ±0.02` | `PARTIAL` | `AUX:143-303`; `AUX:306-422`; QA `03-vellum-1440-ortho.png` | Real Section A–A and third-angle views exist, but there is no miniature sheet preview and the primitive schema has no flange/bolt-hole geometry. |
| 11 | “Issued for fabrication” stamp | Angled verified stamp | `OMITTED-FAKE` | `SHEET:273-310` | No fabrication-release state exists; no stamp is printed. |
| 12 | Sheet-card metadata | `FORMAT: ISO A3 LANDSCAPE`, `SCALE: 1:2.5 METRIC`, `SHEET 1 OF 3` | `PARTIAL` | `SHEET:282-296`; `APP:87-90` | Title block has `FIT` scale, `METRIC (MM)`, `SHEET 01`; no paper format or “1 of N” (honest: no multi-sheet set). |
| 13 | FE stress sheet card | `SH-02: FE Stress Heatmap`, max stress, safety factor, mesh nodes | `MISSING-NEEDS-BACKEND` | Stitch `code.html:115-151`; `SERVER:1-25` | No FEA solver, mesh statistics, or result storage. |
| 14 | FE numeric values | `342.8 MPa`, `2.84 η`, `48,210 NODES` | `OMITTED-FAKE` | Stitch `code.html:131-138` | Placeholder analysis values are not displayed anywhere. |
| 15 | `PRE-COMMIT PENDING` stamp | Angled pending stamp on the FE sheet | `OMITTED-FAKE` | `JRN:221-232` | There is no pre-commit state; the journal uses neutral `SESSION LOG`. |
| 16 | BOM sheet card | `SH-03: Bill of Materials`, item/spec, QTY/alloy rows | `MISSING-NEEDS-BACKEND` | Stitch `code.html:152-185` | No assembly/BOM/fastener model in the primitive or Studio API. |
| 17 | Title-block editor card | Charcoal `Title Block Stamp [DIN 6771]`, `ACTIVE DRAFT` | `PARTIAL` | `SHEET:273-310`; `DRAFT:394-439` | A real, read-only title block sits on the vellum sheet; there is no dark editor or editable fields. |
| 18 | Drawn by | `DRAWN BY: KALA-AI-AUTONOMOUS` | `PARTIAL` | `APP:92-93`; `SHEET:306-309` | The sheet prints `AUTHOR: Kala Studio` and `REV: DRAFT`; the label is author, not drawn-by. |
| 19 | Checked by | `CHECKED BY: CHIEF DRAFTSMAN` | `MISSING-NEEDS-BACKEND` | `SHEET:273-310` | No reviewer identity or approval model exists; printing a fake name is forbidden. |
| 20 | Date | `DATE: 2025-03-31` | `MISSING-NEEDS-BACKEND` | `SHEET:273-310` | No revision/issue date in `TitleBlockData`; the current date is not a drawing-revision date. |
| 21 | Projection | `PROJECTION: THIRD ANGLE ⊕` | `PRESENT` | `APP:90`; `SHEET:290-293` | `3RD ANGLE` is real shell data and is displayed. |
| 22 | Alloy material + Swap | `Ti-6Al-4V GRADE 5`, `SWAP` | `MISSING-NEEDS-BACKEND` | `TYPES:77-97` | No material field, material catalogue, or material-change endpoint. |
| 23 | Finish specification | `FINISH SPEC: Ra 0.8 µm GROUND` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:233-236` | No surface-finish model. |
| 24 | General tolerance | `ISO 2768-mK` | `PARTIAL` | `APP:91`; `SHEET:298-301` | The `TOLERANCE` cell is real but always `—`; the backend exposes no tolerance model. |
| 25 | Hash field | `HASH: 0x4F9B...88A2` | `MISSING-NEEDS-BACKEND` | `TYPES:33-39`; `JRN:148-152` | The journal has an optional real digest string, but the Studio export response has no artifact SHA; no fake hash is printed. |
| 26 | Signature verified | `SIGNATURE: VERIFIED [SHA-256]` | `OMITTED-FAKE` | `JRN:221-232` | No signing/verification exists; no verified mark is displayed. |
| 27 | Vector spooler header | `VECTOR SPOOLER ACTIONS`, `QUEUE: 0 JOBS ACTIVE` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:252-256` | No spooler or job queue. |
| 28 | Send to plotter | `Send to HP Vector Pen Plotter`, `PEN: #2 (0.35mm)` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:257-267` | No plotter integration; the specific printer/pen is placeholder hardware. |
| 29 | Compile PDF / DXF | `Compile Multi-Sheet PDF / DXF`, `ZIP (14.2 MB)` | `MISSING-NEEDS-BACKEND` | `EXP:30`; `CLIENT:198-318` | Only STL and STEP exports exist; no PDF/DXF/ZIP writer. |
| 30 | Dispatch toast | `DISPATCHED TO HP-7475A SPOOLER` / `COMMITTING HPGL...` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:277-284` | Depends on the missing spooler; the exact model/values are fake. |
| 31 | Bottom mobile navigation | CANVAS / PARAMS / AUDIT / SHEETS | `N/A` | `WAVE.md:18-28`; `LAYOUT:11-20` | The product is a desktop 4-zone workbench; those functions live in the zones and Options drawer. |
| 32 | Card corner ticks | Four `+` marks on each sheet card | `PARTIAL` | `SHEET:150-153`; `CSS:1044-1053` | The sheet has four corner pins; the Options cards only one `+`; this folio has no cards. |
| 33 | Card hover / selected states | `SELECTED` tag and hover background on sheet cards | `PARTIAL` | `CSS:470-486`; `SHEET:158-160` | Equivalent hover/active states exist for tabs/buttons; no selectable sheet-card state. |

Counts: 33 rows — `PRESENT` 1 · `PARTIAL` 8 · `MISSING-BUILDABLE` 0 · `MISSING-NEEDS-BACKEND` 13 · `OMITTED-FAKE` 4 · `N/A` 7.

---

## 3. `hash_chain_ledger_audit`

This screen depicts a full cryptographic audit console. The built JournalPanel
is an app-event ledger with an honest chain stamp. The core Python project has
a real hash-chained `EventJournal` (`JOURNAL_PY:39-70, 220-284`), but the
Studio server is deliberately a separate ASGI app with no journal route
(`SERVER:1-25`; only health/integrations/providers/settings/preview/export/
handoff/chat). Therefore most audit-console data is backend work, not fakeable
UI work.

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---------|---------------|--------|-------------------|-------|
| 1 | Fixed app header | `SEC. C4`, `COORD: 39°N / 104°W`, `CALIPER: SYNCED [2H]` | `N/A` | Stitch `code.html:7` | Decorative mobile chrome; the wave has no top header. |
| 2 | Brand / audit label | `KALA PARAMETRICS // Audit`, `SPEC: 8402-AE // REVISION: D.4` | `N/A` | Stitch `code.html:7` | Different product/screen brand; the app's journal header is the functional equivalent. |
| 3 | Coordinate margin strip | `ZONE B-3`, `LEDGER: ED-25519-ARCHIVAL` | `N/A` | Stitch `code.html:9-19` | No zoned ledger identity; decorative sheet coordinates. |
| 4 | Chain-synchronous indicator | Pulsing dot + `CHAIN SYNCHRONOUS` | `OMITTED-FAKE` | `JRN:125-131`; `APP:140` | The app never claims chain synchronization; it shows `SESSION LOG` unless a real backend verifies. |
| 5 | Archival title block | `ARCHIVAL SPEC // LEDGER C7`, `SQLITE v2 // BLK 1,048,291` | `MISSING-NEEDS-BACKEND` | `JOURNAL_PY:39-70`; `SERVER:1-25` | Core journal/block stats exist but are not exposed by the Studio server. |
| 6 | Audit headline | Newsreader italic “IMMUTABLE DRAFTING AUDIT” | `PARTIAL` | `JRN:123-124`; `JRCSS:27-47`; `TOK:44` | Journal header is a small mono uppercase label; no serif audit headline. |
| 7 | Intro / certification copy | “Continuous topological trace … ISO 10303-242 ed.2” | `OMITTED-FAKE` | Stitch `code.html:35-37` | No such certification or trace is claimed. |
| 8 | Chain-integrity badge | `CHAIN INTEGRITY 100% VALIDATED` with verified icon | `PARTIAL` | `JRN:125-131, 221-232` | A real stamp exists and branches verified/broken, but the shell passes `hashChainStatus="unknown"` (`APP:140`), so it reads `SESSION LOG`. |
| 9 | Latency / peers | `LATENCY: 12ms`, `PEERS: 06` | `OMITTED-FAKE` | `JRN:170-177` | No peer/latency telemetry; footer shows the real origin and backend version only. |
| 10 | Tool-class change alert | `TOOL_CLASS_CHANGE DETECTED`, warning icon, `CRIT-A1` | `MISSING-NEEDS-BACKEND` | `TYPES:30-39`; `JRN:38-45` | No policy/authority event kind or backend policy event stream in Studio. |
| 11 | Severity stamp | `CRIT-A1` | `OMITTED-FAKE` | Stitch `code.html:68` | Placeholder severity is not displayed. |
| 12 | Authority transition | `READ_ONLY_INSPECTOR → PARAMETRIC_MUTATOR`, timestamp, `SEC_POLICY: 48-K` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:70-86` | No authority model, policy id, or sign-off workflow. |
| 13 | Alert timestamp / policy | `TIMESTAMP: 14:22:08.411 UTC`, `SEC_POLICY: 48-K` | `OMITTED-FAKE` | Stitch `code.html:72-73` | Placeholder values; no fake timestamp is shown. |
| 14 | Acknowledge action | `ACKNOWLEDGE` button | `MISSING-NEEDS-BACKEND` | Stitch `code.html:88-90` | Needs a policy acknowledgment endpoint and state. |
| 15 | Ledger feed divider | `LEDGER FEED // MONOSPACE CARDS`, `ORDER: DESC`, `N=3 BLOCKS` | `PARTIAL` | `JRN:134-154`; `JRCSS:83-99` | Rows are chronological ascending with autoscroll; no count/order label. `entries.length` is available for a count. |
| 16 | Audit row header | Block `#1048291`, `14:28:19 UTC`, author, angled approval stamp | `PARTIAL` | `JRN:141-154`; `JRCSS:104-124` | Rows show a kind tag, local time, message; no block id, author, or approval stamp. |
| 17 | Operation descriptor | `MUTATE_PARAM [D_BORE: 72.00 → 74.50]`, delta/tolerance | `PARTIAL` | `TYPES:33-39`; `JRN:147` | `JournalEntry` has only kind/message/digest, so rows are free text; no structured operation/delta fields. |
| 18 | SHA-256 row | Full/truncated SHA-256 with label | `PARTIAL` | `JRN:148-152`; `EXP:119-123` | A digest chip renders only when a real digest exists (export rows carry bytes/format, not a SHA). |
| 19 | Copy-hash button | Clipboard icon per hash row | `MISSING-BUILDABLE` | `JRN:148-152`; `TYPES:33-39` | The digest text is already in the DOM; a copy button can be added without backend work. |
| 20 | Simulation matrix card | `STRESS_SOLVER_RUN`, max calculated, material yield, safety factor | `MISSING-NEEDS-BACKEND` | Stitch `code.html:158-177`; `SERVER:1-25` | No FEA/solver results. |
| 21 | Simulation numbers | `284.1 MPa`, `880.0 MPa`, `SAFETY FACTOR 3.097` | `OMITTED-FAKE` | Stitch `code.html:164-175` | Placeholder analysis values are not shown. |
| 22 | Genesis block card | `GENESIS_BLOCK`, `INITIALIZE_GEOMETRY_BASELINE` | `MISSING-NEEDS-BACKEND` | `JOURNAL_PY:70-160`; `SERVER:1-25` | Core journal has a genesis event, but Studio cannot query it. |
| 23 | Genesis details | `18,420 dual-quad nodes`, `CSYS: WGS84 Cartesian Ortho` | `OMITTED-FAKE` | Stitch `code.html:208-210` | No such mesh/CSYS record is displayed. |
| 24 | Microfilm inspection image | Remote archival blueprint image with callouts | `N/A` | Stitch `code.html:224-241`; `WAVE.md:16` | Decorative remote image; no functional meaning and remote assets are forbidden. |
| 25 | Section D-D' overlay | `SEC: D-D'`, `FLANGE THICKNESS: 14.2mm` | `OMITTED-FAKE` | `AUX:48-52`; `AUX:143-303` | The app supports Section A–A only; no D–D′ section or flange thickness exists. |
| 26 | “Verified airworthy” stamp | `VERIFIED AIRWORTHY` | `OMITTED-FAKE` | Stitch `code.html:237-239` | No airworthiness claim; correctly not displayed. |
| 27 | Hard sign-off terminal | `HARD SIGN-OFF TERMINAL`, `HSM-04`, RSA-PSS 4096 fingerprint, `YubiKey 5 FIPS` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:243-266` | No HSM, hardware key, or signing key integration. |
| 28 | Apply pre-commit seal | `APPLY PRE_COMMIT SEAL`, `EMBED 2H GRAPHITE SIGNATURE & ADVANCE HEAD` | `MISSING-NEEDS-BACKEND` | `JOURNAL_PY:70-160`; `SERVER:1-25` | Core journal/policy has checkpoints, but no Studio signing/append endpoint. |
| 29 | Current rev / next block | `SPEC-8402 // D.4`, `1,048,292 [READY]` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:282-291` | No revision or block counter in the Studio. |
| 30 | Encryption / log format | `ENCRYPTION: HARD-SECURED`, `LOG FORMAT: ARCHIVAL-V4` | `OMITTED-FAKE` | Stitch `code.html:292-295` | No such format/security claim is displayed. |
| 31 | Seal success toast | `PRE_COMMIT SEAL ATTESTED`, block/tx | `MISSING-NEEDS-BACKEND` | Stitch `code.html:297-306` | Depends on the missing sign-off endpoint. |
| 32 | Bottom mobile navigation | CANVAS / PARAMS / AUDIT / SHEETS | `N/A` | `WAVE.md:18-28` | Desktop 4-zone layout replaces mobile tabs. |
| 33 | Empty ledger state | (not shown; implied empty feed) | `PRESENT` | `JRN:135-140`; `JRCSS:82-89` | Explicit “No journal events yet…” state. |
| 34 | Jump-to-latest control | (not shown in Stitch) | `PRESENT` | `JRN:156-165, 111-116` | Honest addition for a bounded scroll list. |
| 35 | Kind-coloured tags | Category colour coding | `PRESENT` | `JRN:38-45, 141-154`; `JRCSS:126-155` | Real kinds are `preview/chat/tool/export/connection/system`; not the mock's categories. |

Counts: 35 rows — `PRESENT` 3 · `PARTIAL` 6 · `MISSING-BUILDABLE` 1 · `MISSING-NEEDS-BACKEND` 10 · `OMITTED-FAKE` 10 · `N/A` 5.

---

## 4. `parametric_constraint_matrix`

This screen is a constraint/solver parameter console. The only mapped product
surface is the Options card 01 Inspector; the solver/constraint content has no
backend model in the Studio.

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---------|---------------|--------|-------------------|-------|
| 1 | Coordinate strip | `ZONE [B-3]`, `REF: DIN EN ISO 5457`, `TOPOLOGY ENGINE V4.2` | `N/A` | Stitch `code.html:9-20` | Decorative sheet coordinates/engine label; no zoned audit surface. |
| 2 | Solver-active indicator | Green square + `SOLVER ACTIVE` | `OMITTED-FAKE` | Stitch `code.html:17-18` | No solver runs; the app does not claim one. |
| 3 | Constraint-matrix header | `SUBSYSTEM 08 // FLANGE HOUSING`, italic “Geometric Constraint Matrix”, `CALIBRATED` stamp | `PARTIAL` | `INS:112-117`; `OPT:181`; `CSS:426-439` | A real parameter panel exists under the drawer card; there is no constraint matrix, subsystem, or calibrated stamp. |
| 4 | Solver state | `EQUILIBRIUM (DOF = 0) // 14 Rules Enforced` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:39-42`; `SERVER:1-25` | No DOF/constraint solver or rule engine. |
| 5 | Parameters header / units | `Primary Dimensional Parameters`, `UNITS: SI (mm / deg)` | `PARTIAL` | `INS:209-240`; `TYPES:24` | Parameter editing is present in mm; no angular parameters or SI/deg switch. |
| 6 | Bore parameter card | `TAG: 0x4A`, `Bore Diameter [D_BORE]`, `STABLE [STIFFNESS: 98.4%]` | `MISSING-NEEDS-BACKEND` | `TYPES:77-97`; `INS:209-240` | No bore, stiffness, or tagged parameter model. |
| 7 | Realtime vernier readout | Large `74.500 mm` readout separate from the editor | `PARTIAL` | `INS:47-86`; `CSS:514-527` | The numeric field itself is the readout; slider and field are linked, but there is no separate large live value. |
| 8 | Caliper notch rail | Graduated notch rail with major/minor ticks and filled track | `PARTIAL` | `INS:74-83`; `CSS:524-527` | Native range input; no notch rail or filled track. |
| 9 | Min/max/step readout | `MIN: 68.000 mm`, `Δ 0.025 mm STEP`, `MAX: 82.000 mm` | `MISSING-BUILDABLE` | `TYPES:55-66`; `INS:214-217` | Min/max/step are already defined per primitive and passed to the input; printing them under the slider needs no backend. |
| 10 | Stepper controller | `−` / `+` buttons around `18.250 MILLIMETERS` | `PARTIAL` | `INS:47-87` | Numeric typing works, but there are no ± stepper buttons. |
| 11 | Critical requirement badge | Lock icon + `CRITICAL [BURST REQ]` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:129-132` | No pressure/burst requirement model. |
| 12 | Governing rule | `P_BURST >= 320.0 bar (EN-GJS-400-18)` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:147-150` | No stress/rule evaluation. |
| 13 | Bolt-circle parameter card | `Bolt Pitch Circle [PCD_BOLTS]`, `8x EQUIDISTANT`, PCD 128.000 | `MISSING-NEEDS-BACKEND` | `TYPES:77-97` | No bolt pattern or PCD field in the primitive schema. |
| 14 | Radial offset / stud data | `22.5° (Staggered to Stress Line)`, ISO 4014 M12, preload 48.2 kN | `MISSING-NEEDS-BACKEND` | Stitch `code.html:180-199` | No angular scrubber, stud catalogue, or preload model. |
| 15 | Stress-equilibrium terminal | Dark card, σ_hoop equation, computed tension, margin of safety, hash | `MISSING-NEEDS-BACKEND` | Stitch `code.html:202-234` | No FEA/closed-form stress verification or result hash. |
| 16 | Re-solve action | `Re-Solve Topology & Constraints` primary button | `MISSING-NEEDS-BACKEND` | Stitch `code.html:235-241` | No topology/constraint solver to invoke. |
| 17 | Export STEP / DXF | Secondary `EXPORT STEP / DXF` button | `PARTIAL` | `EXP:181-188`; `EXP:30` | STEP export exists in card 02 with real receipt; DXF is not supported by the backend. |
| 18 | Generate report | `GENERATE REPORT` button | `MISSING-NEEDS-BACKEND` | `SERVER:1-25` | No report-generation endpoint. |
| 19 | Sheet metadata footer | `DRAWING CODE KP-8402-AE-04`, `SCALE RATIO 1:2.50 METRIC`, `LAST CALIPER 04:12:09 UTC` | `PARTIAL` | `SHEET:282-310`; `APP:81-96` | Title block has sheet/scale/units/projection; no drawing code or last-calibration time. |
| 20 | Parameter-card corner crosses | Four `+` per parameter card | `PARTIAL` | `CSS:1044-1053`; `SHEET:150-153` | Only the sheet has four pins; drawer cards have a single top-left `+`. |
| 21 | Sparse tag addresses | `TAG: 0x4A`, `0x4B`, `0x4C` | `OMITTED-FAKE` | Stitch `code.html:63,123,161` | No fake parameter addresses are displayed. |
| 22 | Bottom mobile navigation | CANVAS / PARAMS / AUDIT / SHEETS | `N/A` | `WAVE.md:18-28` | Desktop 4-zone layout; no mobile tab shell. |
| 23 | Validation error state | (not in Stitch; implied by numeric editing) | `PRESENT` | `INS:89-96, 120-124, 529-541` | Real inline range/zero validation with `role="alert"` and red field border. |
| 24 | Hover / active / focus states | Physical pressed controls | `PRESENT` | `BASE:85-92`; `CSS:470-486`; QA REPORT §4 | Focus ring and active inversion are implemented; no separate hover treatment on every stepper (steps are missing). |

Counts: 24 rows — `PRESENT` 2 · `PARTIAL` 8 · `MISSING-BUILDABLE` 1 · `MISSING-NEEDS-BACKEND` 9 · `OMITTED-FAKE` 2 · `N/A` 2.

---

## 5. `drafting_canvas_workbench`

This is the closest Stitch screen to the shipped drafting sheet. It supplies
the viewport furniture, tool strip, and the tactile dock idea.

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---------|---------------|--------|-------------------|-------|
| 1 | Fixed app header | `SEC. C4`, `COORD: 39°N / 104°W`, `CALIPER: SYNCED [2H]` | `N/A` | Stitch `code.html:7`; `WAVE.md:27` | Decorative chrome; the wave removes the top header. |
| 2 | DIN A4 metadata strip | `SHEET REF: KL-9021-M`, `TOL: ±0.025mm`, `GRADE: 4B CARBON` | `PARTIAL` | `SHEET:156-186`; `SHEET:273-310` | Sheet tab + title block carry sheet/scale/units; no strip, paper format, grade, or tolerance (tolerance is `—`). |
| 3 | Specific metadata values | `KL-9021-M`, `±0.025mm`, `4B CARBON` | `OMITTED-FAKE` | `APP:83-91` | Placeholder values are not printed; the sheet says `SHEET 01`, `FIT`, `—`. |
| 4 | Quick tool strip | Caliper, Compass, T-Square, Eraser Shield, 2H Grid | `PARTIAL` | `SHEET:188-238`; `DRAFT:245-298` | A real 4-button palette (reset, dims, A–A, grid) exists; the five mock tools differ. |
| 5 | Caliper measure tool | Active `CALIPER` tool | `MISSING-NEEDS-BACKEND` | Stitch `code.html:32-36` | No measurement/picking API or cursor span tool. |
| 6 | Compass / arc tool | `R:42.5` compass | `MISSING-NEEDS-BACKEND` | Stitch `code.html:37-41` | No sketch/arc editing capability. |
| 7 | T-square / 0° lock | `0° LOCK` orthographic tool | `MISSING-NEEDS-BACKEND` | Stitch `code.html:42-46` | Camera orbit only; no orientation lock/constraint. |
| 8 | Eraser shield | `SHIELD` tool | `MISSING-NEEDS-BACKEND` | Stitch `code.html:47-51` | No editable sketch geometry. |
| 9 | 2H grid toggle | `2H GRID` active tool | `PRESENT` | `SHEET:226-237`; `BAR:73`; `STORE:87` | Real `showGrid` toggle in both sheet palette and bottom bar. |
| 10 | Viewport perimeter | Corner crosses, `1–8` top, `A–F` left | `PRESENT` | `SHEET:85-154`; `DRAFT:57-154` | All four coordinate edges, boundary ticks, and corner pins. |
| 11 | Floating caliper ruler | `VERNIER SPAN: 74.500 mm`, `DIV: 0.05` overlay | `MISSING-NEEDS-BACKEND` | Stitch `code.html:74-83` | No measurement span; the value is fabricated in the mock. |
| 12 | Fine vellum grid / centerlines | Fine grid, long-short centerlines | `PRESENT` | `DRAFT:34-44`; `SCENE:361-388` | Grid pattern + grain; axes are drafter centre lines (X solid, Y/Z dashed). |
| 13 | Live vector geometry | Full drafting vector canvas | `PRESENT` | `SHEET:240-253`; `SCENE:188-200`; QA shots | Real WebGL render of the active primitive, not a static SVG backdrop. |
| 14 | Section hatch / leaders / A–A | 45° hatch, dimension leaders, arrowheads, A–A flags | `PARTIAL` | `AUX:143-303`; `SHEET:255-261`; `SCENE:721-740` | A–A flags, hatch (AuxView), and real dimension callouts exist; the main sheet cut face is a plane, not a hatched 2D section. |
| 15 | Pre-commit geom-verified stamp | `[ PRE_COMMIT: GEOM_VERIFIED ]`, stamp id/date | `OMITTED-FAKE` | `JRN:221-232`; `SHEET:273-310` | No verification stamp; the app never claims pre-commit verification. |
| 16 | Lead-hardness dock | 4B / 2H / HB calibration selector, `ISO 9175-1` | `N/A` | Stitch `code.html:182-224` | Decorative drafting metaphor with no functional mapping; adding fake lead selection would not serve the product. |
| 17 | X / Y / θ cursor telemetry | Live pointer readout `+142.85mm`, `-88.10mm`, `37.5°` | `MISSING-BUILDABLE` | `SCENE:206-225`; `DRAFT:358-378` | The camera and current bounds are already in the renderer; a pointer listener can project screen coordinates to the model plane without backend work. |
| 18 | Pin Datum action | `PIN DATUM [D1]` | `MISSING-NEEDS-BACKEND` | Stitch `code.html:242-246` | No datum/reference-geometry model. |
| 19 | Execute / solver action | `EXECUTE (F5)`, converging/converged feedback | `MISSING-NEEDS-BACKEND` | Stitch `code.html:247-251` | No solver execution; the mock's `0.002ms` result is fake and correctly absent. |
| 20 | Crypto proof ledger bar | `SHA: 8f4a...91bc // CHAIN_LINKED`, `MERKLE ROOT SYNC` | `PARTIAL` | `JRN:148-152`; `JRN:221-232` | A real digest slot exists in journal rows, but no SHA/Merkle/chain is displayed without backend data. |
| 21 | Bottom mobile navigation | CANVAS / PARAMS / AUDIT / SHEETS | `N/A` | `WAVE.md:18-28` | Desktop 4-zone layout. |
| 22 | Section A–A flags | `A` flags on both sides + line | `PRESENT` | `SHEET:255-261`; `DRAFT:310-331` | Redline A–A rule/flags render only when the section is on. |
| 23 | Dimension overlay toggle | Dimension callouts | `PRESENT` | `SHEET:201-215`; `BAR:75`; `SCENE:603-620` | Real toggle hides/shows the Three.js dimension overlay and HTML labels. |
| 24 | Reset view | Re-frame camera | `PRESENT` | `SHEET:189-200`; `BAR:77-85`; `SCENE:176-182` | Direct camera reset via `viewResetToken`; no remount. |
| 25 | Orbit / zoom note | — | `PRESENT` | `SHEET:263-265`; `SCENE:221-225` | Honest “DRAG ORBIT · SCROLL ZOOM” note and OrbitControls. |
| 26 | Empty/no-mesh state | No mesh yet | `PRESENT` | `SCENE:188-200`; `AUX:311-322` | Honest “No mesh” source badge and “Run a preview” ortho empty state. |

Counts: 26 rows — `PRESENT` 9 · `PARTIAL` 4 · `MISSING-BUILDABLE` 1 · `MISSING-NEEDS-BACKEND` 7 · `OMITTED-FAKE` 2 · `N/A` 3.

---

## 6. `STITCH-VELLUM-PROMPT.md` §2–3 element list

This table audits the prompt's explicitly named elements, including those that
also appear in the master-workspace screens owned by another auditor. It is
the requirement checklist for the wave, not a duplicate of the screen tables.

### §2 — master workspace

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---------|---------------|--------|-------------------|-------|
| 1 | Left column geometry | 340 px wide, full height, agent chat | `PRESENT` | `LAYOUT:11-34`; `TOK:48`; QA REPORT §2 | 340 px at 1440, 280 px below 1400. |
| 2 | Bound calculation-pad styling | Taped/ruled margin, bound pad | `PARTIAL` | `CHATCSS:1-20, 156-215`; QA `01-…default.png` | Panel surface, ruled starter tabs, and stamped header are present; no physical binding/spine. |
| 3 | Header stamp | `KALA // MECHANICAL DESIGN AGENT` | `PRESENT` | `CHAT:231-238`; `CHATCSS:23-68` | Exact title in a drafted stamp. |
| 4 | Connection dot / label | Green dot `LOCAL LOOPBACK` | `PARTIAL` | `CHAT:235-238`; `CHATCSS:69-105` | Real dot and real state label (`Desktop bridge` / `Browser fallback` / `Connecting` / `Disconnected`); not the generic “LOCAL LOOPBACK”. |
| 5 | Starter chips | `[Size L-Bracket]`, `[Size Stepped Shaft]`, `[Housing Envelope]`, `[STEP Handoff]` | `PRESENT` | `CHAT:52-77, 271-287` | Labels match; four starters seed the composer without auto-send. |
| 6 | Taped paper-tab styling | Tape strip and slight rotation | `PRESENT` | `CHATCSS:173-217` | Tape strip via `::before`, alternating ±0.5° rotation. |
| 7 | Scrollable conversation thread | Scrollable turn thread | `PRESENT` | `CHAT:309-348`; `CHATCSS:304-367` | Sticky to newest unless scrolled up. |
| 8 | User / agent message cards | Ruled-note cards with role/time | `PRESENT` | `CHAT:322-335`; `CHATCSS:312-365` | Role `You`/`Agent`, local time, real text. |
| 9 | Agent inline result table | Envelope / nominal volume / material table inside agent answer | `PARTIAL` | `CHAT:288-307`; chat-journal REPORT gap 1 | The mono table is the real **active workbench geometry**, not a parsed agent result; the backend `ChatResponse` has only text/provider/model/offline. |
| 10 | Material annotation | `Material: 6061-T6 Surrogate` | `MISSING-NEEDS-BACKEND` | `TYPES:202-207` | No material field in the chat response or primitive model. |
| 11 | B-Rep annotation | Prussian “Parametric B-Rep ready. Section A-A updated…” | `PARTIAL` | `CHAT:288-307`; `SHEET:255-261` | Section updates are real; no B-Rep state is claimed, and no annotation is added. |
| 12 | Composer text area | “Describe a part, tolerance, or revision…” | `PRESENT` | `CHAT:420-431` | Placeholder is “Describe geometry, tolerance, or prompt…”. |
| 13 | Pencil / mic / attachment icons | Three multimodal input icons | `MISSING-NEEDS-BACKEND` | `CHAT:432-446` | No sketch capture, voice input, or attachment endpoint; dead icons were not added. |
| 14 | Remote egress consent | Checkbox stamp | `PRESENT` | `CHAT:388-407`; `CHATCSS:481-531` | Appears only for remote providers; real per-request consent gate. |
| 15 | Draft button | Charcoal `[ Draft Part ↵ ]` | `PRESENT` | `CHAT:432-435`; `CHATCSS:540-556` | `[Draft Part Geometry ↵]`, disabled until the real send preconditions pass. |
| 16 | Center column at top | 800 px, no top header | `PRESENT` | `LAYOUT:11-20, 36-42`; QA REPORT §2 | Center starts at the top edge; no header/footer. |
| 17 | Full-bleed vellum | `#F6F3EB` sheet | `PRESENT` | `TOK:13`; `DRAFT:34-44` | Sheet surface plus grid/grain. |
| 18 | Technical border | Double 1 px drafted border | `PRESENT` | `DRAFT:57-70` | 1.5 px outer + 0.5 px inner (the wave's drafted border). |
| 19 | Zone coordinates | A–F, 1–8 with ticks | `PRESENT` | `SHEET:71-153`; `DRAFT:72-154` | All four sides and boundary ticks. |
| 20 | Pencil grid | Isometric/orthographic grid, 50 mm major / 10 mm minor | `PRESENT` | `TOK:39`; `DRAFT:34-44`; QA `01-…png` | SVG 50 px block with 10 px subdivisions + `+` ticks. |
| 21 | Centerpiece part | Precision L-bracket | `PARTIAL` | `TYPES:5-6`; QA REPORT §6.1 | Only box/cylinder/sphere primitives exist; the active primitive is rendered, not a faked bracket. |
| 22 | Hybrid 2D/3D toggle | `[ ✎ 2D Pencil Vector (Active) / ⬡ 3D Shaded Clay ]` | `PRESENT` | `SHEET:163-185`; `DRAFT:200-243` | Bound to `renderMode`; the pill reads `2D Pencil Vector` / `3D Clay`. |
| 23 | Visible + hidden edges | 2B visible, dashed 2H hidden pass | `PRESENT` | `SCENE:550-575`; P2a REPORT | Real `EdgesGeometry`, dashed `GreaterDepth` hidden pass. |
| 24 | Centerline axes | Redline centre lines | `PARTIAL` | `SCENE:361-388` | X/Y/Z axes are drawn through the model origin; no bolt/bore center marks (no bore geometry). |
| 25 | Redline dimensions | Hand-lettered dimension callouts | `PRESENT` | `SCENE:690-740`; `DRAFT:358-378`; QA `01-…png` | Real bounds dimensions projected to HTML labels while orbiting. |
| 26 | Specific callouts | `60.00 mm`, `Ø 10.00 H7`, `R 4.0` | `MISSING-NEEDS-BACKEND` | `TYPES:77-97` | No holes, fillets, or tolerance class in the primitive contract. |
| 27 | Cutting plane A–A | Terracotta `A ——— A` | `PRESENT` | `SHEET:255-261`; `SCENE:780-820` | Real mid-plane `z = 0` cut and flags. |
| 28 | Title block watermark | `PART: KALA-BRKT-01 / SCALE: 1:1 / UNITS: MM / REV: 03` | `PARTIAL` | `SHEET:273-310`; `APP:81-96` | Real part id/title/scale/units/rev, but values are `STUDIO-<KIND>`, `FIT`, `METRIC (MM)`, `DRAFT`; 1:1 and rev 03 are not claimed. |
| 29 | Bottom ruler bar | 48 px brass/boxwood scale ruler | `PRESENT` | `LAYOUT:36-42, 227-240`; `TOK:33-34`; QA REPORT §4 | Exactly 48 px, boxwood tokens, drafted top border. |
| 30 | 2D/3D switch | Segmented 2D/3D | `PRESENT` | `BAR:57-72` | Real `renderMode` switch. |
| 31 | Pencil grid toggle | Grid toggle | `PRESENT` | `BAR:73`; `SHEET:226-237` | Real `showGrid`. |
| 32 | Center-axes toggle | Axes toggle | `PRESENT` | `BAR:74`; `SCENE:603-620` | Real `showAxes`; axes rebuilt on theme change. |
| 33 | Redline-dims toggle | Dims toggle | `PRESENT` | `BAR:75`; `SCENE:603-620` | Hides the dimension group and labels. |
| 34 | Section-cut toggle | Section A–A | `PRESENT` | `BAR:76`; `SHEET:216-225` | Real `showSection`; also removes flags/cut note. |
| 35 | Reset view | Reset camera | `PRESENT` | `BAR:77-85`; `SCENE:587-596` | Re-frames the live camera. |
| 36 | Live nominal readout | `ENVELOPE`, `VOL`, `AREA`, `[✓ B-Rep Verified]` | `PARTIAL` | `BAR:88-97`; QA `01-…png` | ENV/VOL/AREA are real and labelled with source (`server` / `local est.`); no B-Rep-verified stamp. |
| 37 | B-Rep verified badge | Green verified stamp | `OMITTED-FAKE` | `SCENE:195-199`; QA REPORT §5 | The app shows “Server verified mesh” only when a real server mesh exists, not a B-Rep claim. |
| 38 | Export STEP | `[Export STEP]` | `PRESENT` | `BAR:100-107` | Opens Options card 02; real export panel. |
| 39 | FreeCAD Live | `[FreeCAD Live]` | `PARTIAL` | `BAR:108-116, 168-173` | Shows the real registry status (`neutral`, `no API`, `unset`, `—`); no live bridge/PID. |
| 40 | Options trigger | `[ ⚙ Options & Tools ]` | `PRESENT` | `BAR:117-124`; `OPT:42` | Opens the modal on card 01. |
| 41 | Right column | 320 px split 250 / remainder | `PRESENT` | `LAYOUT:11-20, 44-50`; `TOK:49-51`; QA REPORT §2 | AuxView 250 px, journal fills the rest. |
| 42 | Pinned detail-loupe framing | Auxiliary view framed like a pin-up detail card | `PARTIAL` | `AUX:80-140`; `DRAFT:443-451` | Panel/tabs/footer exist; no pin/loupe or drafting-detail border. |
| 43 | Aux tabs | `Section A-A (Active) / Ortho 3-View / Tolerance Map` | `PRESENT` | `AUX:48-52, 82-102` | `Tolerance` label, not `Tolerance Map`; all three tabs real. |
| 44 | Section A–A detail | Bore section, 45° ANSI hatch, wall-thickness arrows, bore depth | `PARTIAL` | `AUX:143-303`; P2a REPORT | Real primitive section + hatch + width/height dimension arrows; no bore/wall/depth because the schema has no holes. |
| 45 | Journal header | `AGENT & KERNEL JOURNAL` | `PRESENT` | `JRN:123-131` | Exact heading. |
| 46 | Hash-chain stamp | `[HASH-CHAIN OK]` | `PARTIAL` | `JRN:125-131, 221-232`; `APP:140` | Stamp exists and supports verified/broken, but the shell passes `unknown`; it honestly reads `SESSION LOG`. |
| 47 | Journal log stream | Scrollable monospace ledger | `PRESENT` | `JRN:133-155`; `JRCSS:60-124` | Bounded list, sticky autoscroll, jump control, `aria-live`. |
| 48 | Kernel event categories | `plan.created`, `sketch.new`, `partdesign.pad`, `pocket`, `ocp.brepgprop`, `policy.checkpoint`, `freecad.bridge` | `PARTIAL` | `TYPES:30`; `JRN:38-45` | Real kinds are `preview/chat/tool/export/connection/system`; no agent tool-call protocol exists, so no fabricated kernel steps. |
| 49 | Log line format | time + `[category]` + message + digest | `PRESENT` | `JRN:141-154` | Tag, time, message, optional real digest chip. |

### §3 — Options & Engineering drawer

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---------|---------------|--------|-------------------|-------|
| 50 | Modal overlay with 4 cards | Options modal, four drafted wireframe cards | `PRESENT` | `OPT:130-236`; QA `06-…options.png` | Modal with four cards; see screen 1 for the grid-density differences. |
| 51 | Card 1 shape selector | Box / Cylinder / Sphere / Custom Script | `PARTIAL` | `INS:143-173` | Three real primitives; Custom Script missing (schema). |
| 52 | W/H/D/Wall-Bore inputs | Width, Height, Depth, Wall/Bore in mm | `PARTIAL` | `INS:209-240`; `TYPES:77-97` | W/H/D present; wall/bore absent from the schema. |
| 53 | Vernier inputs + sliders | Tactile ruler sliders | `PARTIAL` | `INS:47-86`; `CSS:524-527` | Native range inputs; ticks/thumb/tooltip not built. |
| 54 | OpenCascade estimate + drift + footnote | Volume, area, bounds drift, honesty note | `PARTIAL` | `INS:184-202, 248-289` | Volume/area/envelope/envelope volume + footnote are real; drift is a warning, not a green stamp. |
| 55 | Card 2 target dropdown | FreeCAD / SOLIDWORKS / Fusion / ANSYS / Abaqus | `PRESENT` | `EXP:143-153`; `INT:19-45` | Real registry, honest statuses. |
| 56 | Format pills | STEP / STL / Native FCStd | `PARTIAL` | `EXP:30, 154-167` | STL/STEP only; no FCStd. |
| 57 | Export & Stamp button | Primary stamped action | `PARTIAL` | `EXP:181-188` | Real “Download STEP/STL”; no stamp/verification claim. |
| 58 | Receipt with SHA | Saved file, bytes, `sha256 verified` | `PARTIAL` | `EXP:189-193`; `CLIENT:287-293` | Filename/bytes are real; no SHA in the response, so no hash line. |
| 59 | Provider rows / status | Local helper, OpenRouter, OpenAI statuses | `PRESENT` | `PRV:76-167`; QA `06b-…png` | Real rows and configured state. |
| 60 | Local helper ACTIVE / OFFLINE | `[ ACTIVE · OFFLINE ]` | `PARTIAL` | `PRV:92-96` | Shows configured / not configured; no per-provider active/offline claim. |
| 61 | Masked key + Save/Clear | Masked key, Save, Clear | `PARTIAL` | `PRV:119-150` | Secure empty password input; save/clear real. |
| 62 | OpenAI + egress toggle | OpenAI row + egress allowlist | `PARTIAL` | `PRV:76-167`; `CHAT:388-407` | OpenAI row real; egress consent is per-send in chat, not a drawer allowlist. |
| 63 | Card 4 origin input | Editable origin | `PARTIAL` | `OPT:290-293`; `CONN:57-70` | Read-only fact in the card; editable in the BootGate fallback. |
| 64 | Token input | Masked bearer token | `OMITTED-FAKE` | `OPT:311-313`; `CONN:71-83` | Never displayed in the drawer; token stays in the connection form only. |
| 65 | Connect Loopback | Primary connect | `PARTIAL` | `CONN:51-90`; `OPT:315-324` | Connect lives in the BootGate; card 04 offers reconnect/retry. |
| 66 | Retry Bridge | Secondary retry | `PRESENT` | `OPT:315-324` | Real retry + journal entry. |
| 67 | In-memory privacy notice | Token never persisted | `PRESENT` | `OPT:221-224` | Explicit security copy. |

Counts: 67 rows — `PRESENT` 37 · `PARTIAL` 25 · `MISSING-BUILDABLE` 0 · `MISSING-NEEDS-BACKEND` 3 · `OMITTED-FAKE` 2 · `N/A` 0.

---

## 7. `tactile_drafting_graphite_workbench/DESIGN.md` — tokens, type, spacing

This table checks whether the design-system tokens, typography scale, and
spacing are applied, as the assignment requests. The wave's `WAVE.md:30-32`
tokens are the canonical palette; DESIGN.md's exact surface values therefore
differ by design and are noted as such.

| # | Element | Stitch detail | Status | Evidence / source | Notes |
|---|---------|---------------|--------|-------------------|-------|
| 1 | Vellum sheet colour | Warm archival canvas `#F6F3EB` (WAVE) | `PRESENT` | `TOK:13`; QA screenshots | Applied to the center sheet. |
| 2 | Panel / margin colour | Alabaster `#EFECE4` | `PRESENT` | `TOK:14-15` | Used for chat/journal/aux panels. |
| 3 | Card / secondary surface | DESIGN.md layered vellum card `#EAE5D9` | `PARTIAL` | `TOK:16`; QA screenshots | App uses a slightly lighter card `#FAF8F2`; same tonal role, not the exact design value. |
| 4 | Hairline | `#C8C2B6`, 0.5 px | `PRESENT` | `TOK:23, 53`; `DRAFT:57-70` | Used throughout. |
| 5 | Primary ink | `#23211E` graphite | `PRESENT` | `TOK:17` | Applied to text and borders. |
| 6 | Construction / 2H | DESIGN.md `#B5AFA2`; WAVE `#7A756C`; app `#645F55` | `PARTIAL` | `TOK:18-22`; QA REPORT §2.2 | Deliberate AA darkening for small text; same graphite family, not the exact design value. |
| 7 | Grid major / minor | `#DFDBD0` / `#EAE6DC` | `PRESENT` | `TOK:24-25, 39`; `DRAFT:34-44` | Exact values in the SVG grid. |
| 8 | Redline line | `#C84B31` | `PRESENT` | `TOK:26`; QA REPORT §2.2 | Dimensions/cut lines unchanged. |
| 9 | Redline text | `#C84B31` (design) vs `#B8432B` (app) | `PARTIAL` | `TOK:27-30` | Added `--redline-ink` for AA; line colour stays exact. |
| 10 | Prussian blue | `#2B5B84` | `PRESENT` | `TOK:31`; QA REPORT §2.2 | Active/selection/focus accent. |
| 11 | Stamp green | `#2A6B4C` | `PRESENT` | `TOK:32` | Server-verified/status green. |
| 12 | Slate night palette | `#0A0A0B` sheet, `#121214` panel, `#F5F5F7` ink, `#26262A` hairline, `#DD5A3E` redline | `PRESENT` | `TOK:57-80`; QA `08-slate-…png` | Matches the night_slate export values; slate Prussian is lightened to `#9ECBFA` for AA. |
| 13 | DESIGN.md exact surfaces | `#FCF9F2` / `#EAE5D9` / `#F4F1EA` | `PARTIAL` | `TOK:13-16` | The app uses the WAVE palette (`#F6F3EB` etc.) as instructed; DESIGN.md is a tone reference. |
| 14 | Space Grotesk UI family | Technical UI lettering | `PRESENT` | `TOK:43`; `MAIN` font imports; QA REPORT §2.1 | Local `@fontsource` files, no remote URL. |
| 15 | Newsreader serif titles | Newsreader italic for sheet headers/notes | `PARTIAL` | `TOK:44`; `CSS:1103-1126`; `DRAFT:598-619` | Used for italic notes and empty states, not for major headings. |
| 16 | JetBrains Mono numbers | All dims/logs/hashes | `PRESENT` | `TOK:45`; `SHEET:273-310`; `JRN:60-175` | Applied to title block, readouts, journal. |
| 17 | Tabular figures | `tnum` for all numerics | `PRESENT` | `BASE:24`; `DRAFT:377, 538`; `JRN:82-88` | Global `font-feature-settings: 'tnum' 1`. |
| 18 | Headline scale | 38 / 28 / 22 px Newsreader | `PARTIAL` | `DESIGN.md:51-74`; `BASE:22`; `CSS:937-941` | App headers are 10–11 px uppercase mono/grotesk; no 38/28/22 hierarchy. |
| 19 | Body scale | 16 / 14 / 12 px | `PARTIAL` | `DESIGN.md:75-89`; `BASE:22`; `JRN:122-124` | Base is 13 px; panel body is 9–12 px. |
| 20 | Label scale | 13 / 11 / 9 px mono | `PARTIAL` | `DESIGN.md:90-107`; `CSS`/`CHATCSS`/`JRCSS` counts | Most labels are 7–9 px; only a few reach 11–13 px. This is the largest typographic deviation. |
| 21 | Spacing base | Strict 4 px increment, tokens xs/sm/md/lg/xl = 4/8/12/20/32 | `PARTIAL` | `DESIGN.md:108-117`; `CSS:900-1208`; `CHATCSS:20-30` | No `--space-*` tokens are used; values are hardcoded (1, 2, 3, 5, 6, 7, 8, 9, 10, 14, 24 px). |
| 22 | Gutter / margin | 12 px gutter, 16 px margin | `PARTIAL` | `DESIGN.md:109-112`; `CSS:917, 1027`; `CHATCSS:29` | Drawer uses 10/14 px, chat 9/12 px, sheet 24 px; not the design-system scale. |
| 23 | Zero radius | All cards/buttons/tabs/inputs = 0 | `PARTIAL` | `TOK:52`; `BASE:49-72`; `CSS:250, 1033-1041` | `--radius: 2px` is applied to base buttons/inputs; many components override to 0. |
| 24 | 1 px / 0.5 px linework | 1 px borders, 0.5 px dividers | `PRESENT` | `TOK:53-54`; `DRAFT:57-70`; `CSS:1033-1042` | Applied consistently. |
| 25 | No shadows / gradients | Flat drafting depth | `PRESENT` | `grep box-shadow/linear-gradient` → 0 matches; `CSS:1-6` | Flat borders only; WAVE rule. |
| 26 | Hard-offset Level-2 stroke | `2px 2px 0 #181816` on floating rules | `N/A` | `WAVE.md:32` | WAVE explicitly forbids drop shadows; the flat treatment is the wave rule. |
| 27 | Level-3 charcoal dock | `#1E1E1C` terminal/dock surface | `PARTIAL` | `JRCSS:1-20`; `LAYOUT:44-50` | Journal is a light panel in vellum (dark only in the slate theme); no dedicated charcoal dock. |
| 28 | Corner crosshairs | `+` at four corners of critical containers | `PARTIAL` | `SHEET:150-153`; `CSS:1044-1053` | Four pins on the sheet; only one top-left `+` per drawer card; none on chat/aux/journal. |
| 29 | Numeric field treatment | Inset surface, right-aligned mono value, hard unit tag | `PARTIAL` | `INS:47-73`; `CSS:514-522` | Right-aligned mono yes; unit is in the label (`mm`), not a separate hard tag. |
| 30 | Caliper slider | 1 px track, tick marks, brass/graphite triangle thumb, live tooltip | `PARTIAL` | `INS:74-83`; `CSS:524-527` | Native range input only. |
| 31 | Checkbox convention | 14×14 sharp box, checked = graphite X | `PARTIAL` | `BASE:81-83`; `CHAT:393-397` | Native checkbox with `accent-color`; no custom X mark. |
| 32 | Radio convention | 14×14 square with 6×6 fill | `PARTIAL` | `INS:149-173`; `CSS:462-486` | Primitive picker uses segmented buttons with `role="radio"`; no square indicator. |
| 33 | Segmented toggle | `[OFF / ON]` two-cell switch | `PRESENT` | `BAR:57-72`; `CSS:250-285`; `OPT:249-269` | 2D/3D mode and theme switch are segmented. |
| 34 | Approval stamp | Irregular dashed 1.5 px red border, −2° tilt | `PARTIAL` | `CSS:1202-1208` | Footer stamp is a flat 0.5 px solid box, no tilt/dashed border. |
| 35 | Hash-chain audit badge | Green mono tag, verified icon, truncated SHA | `PARTIAL` | `JRN:125-131`; `JRN:148-152`; `JRCSS:42-67` | Chain stamp exists (neutral by default); digest chip exists only when a real digest is supplied; no icon. |
| 36 | Title block | 1.5 px heavy box, quadrants, mono metadata | `PRESENT` | `SHEET:273-310`; `DRAFT:394-439` | Real drafted title block on the sheet. |
| 37 | Contrast / legibility | AA for small text | `PRESENT` | QA REPORT §2.2; `TOK:18-30` | QA's browser audit returned zero failing rows in both themes after the AA adjustment. |

Counts: 37 rows — `PRESENT` 17 · `PARTIAL` 19 · `MISSING-BUILDABLE` 0 · `MISSING-NEEDS-BACKEND` 0 · `OMITTED-FAKE` 0 · `N/A` 1.

---

## 8. `a_realistic_high_fidelity_desktop_screenshot_of_siemens_solid_edge_cad_software` — suggestions only

The image is an image-only reference; it is not a target screen. The following
CAD-workbench conventions it shows are absent from Kala Studio. These are
**suggestions**, not defects and not counted in the status totals.

| # | Solid Edge convention | Why it would help Kala Studio | Where it would land |
|---|-----------------------|-------------------------------|---------------------|
| S1 | Ribbon with named command groups and text labels | Makes the tool set discoverable beyond icon-only palettes; current sheet palette has four 28 px icon buttons. | `SHEET:188-238`, `BAR:56-125` |
| S2 | Command search / Quick Access toolbar | A single “find a command” entry (e.g. Ctrl+K) would expose Options cards, export, preview, and theme without hunting. | new `components/shell/CommandPalette.tsx` |
| S3 | PathFinder feature tree | Kala shows a flat parameter form; a read-only model tree (primitive → parameters → preview source) would explain what is editable and what is derived. | `INS:98-135`; new tree component |
| S4 | Part/assembly tree with parts list | Out of scope for a single primitive, but a “part” identity in the sheet tab would make the title block feel like a real document. | `SHEET:156-162`; `APP:81-96` |
| S5 | ViewCube and corner axis triad | The sheet has center axes but no orientation reference; a small triad/ViewCube helps after orbiting. | `SCENE:361-388`; new overlay in `SHEET:240-253` |
| S6 | Named view presets (front/top/right/iso) | The AuxView shows third-angle projections but the main camera has no preset; presets make the drafting view reproducible. | `SCENE:206-225`; `SHEET:188-238` |
| S7 | Display-mode selector (shaded / wireframe / visible edges) | The store already has `MaterialKind = 'default' / 'matte' / 'wireframe'` but no UI exposes it. | `STORE:83-91`; `BAR:56-125` |
| S8 | Status/command prompt line | A transient command/status line (e.g. “preview sent”, “export failed”) would complement the journal and explain disabled actions. | `BAR:88-97`; `JRN:170-177` |
| S9 | Cursor coordinate readout | Model-space X/Y/Z under the pointer is a standard drafting aid and is computable from the existing camera and bounds. | `SCENE:206-225`; new sheet footer |
| S10 | Zoom slider + fit view | Reset View exists; a visible zoom scale and fit command would match CAD expectations. | `BAR:77-85`; `SCENE:221-225` |
| S11 | Undo / redo for parameter edits | Parameter changes are immediate; an undo stack would reduce the cost of experimenting with the sliders. | `STORE:399-425`; `INS:159-165` |
| S12 | Measurement / section commands in the ribbon | Section A–A is already a first-class toggle; a measurement tool is the natural next command. | `SHEET:188-238`; `AUX:143-303` |

---

## Total counts by status

| Screen | Rows | `PRESENT` | `PARTIAL` | `MISSING-BUILDABLE` | `MISSING-NEEDS-BACKEND` | `OMITTED-FAKE` | `N/A` |
|--------|-----:|----------:|----------:|--------------------:|------------------------:|----------------:|----:|
| 1. Options drawer | 53 | 18 | 20 | 1 | 4 | 7 | 3 |
| 2. Drawing sheets / title blocks | 33 | 1 | 8 | 0 | 13 | 4 | 7 |
| 3. Hash-chain ledger audit | 35 | 3 | 6 | 1 | 10 | 10 | 5 |
| 4. Parametric constraint matrix | 24 | 2 | 8 | 1 | 9 | 2 | 2 |
| 5. Drafting canvas workbench | 26 | 9 | 4 | 1 | 7 | 2 | 3 |
| 6. STITCH prompt §2–3 | 67 | 37 | 25 | 0 | 3 | 2 | 0 |
| 7. DESIGN.md tokens/type/spacing | 37 | 17 | 19 | 0 | 0 | 0 | 1 |
| **Total** | **275** | **87** | **90** | **4** | **46** | **27** | **21** |

The high `MISSING-NEEDS-BACKEND` and `OMITTED-FAKE` counts are expected:
the audit screen, constraint matrix, FEA sheets, plotter, and title-block
metadata come from a different product concept, and WAVE invariant 2 forbids
displaying their placeholder values. They are not UI regressions by
themselves; the `PARTIAL` rows are where the built UI can still improve.

---

## Top gaps (ranked by look + usability)

1. **Typography scale is not applied** (`PARTIAL`, screen 7 #18–20).
   DESIGN.md asks for 38/28/22 display, 16/14/12 body, 13/11/9 labels; the
   app uses 6.5–13 px with 7–9 px labels everywhere (`BASE:14-27`; component
   CSS count: 28×7.5 px, 44×8 px). Headings are 10–11 px uppercase rather
   than Newsreader display. This is the largest visual divergence.
   Change: add a type scale to `studio/src/styles/tokens.css` and apply it in
   `base.css`, `studio.css`, `chat.css`, `journal.css`, `drafting.css`.
2. **The vernier/caliper slider is a native range input** (`PARTIAL`, screens
   1 #21, 4 #7–10, 7 #30). The Stitch/design system calls for a 1 px track,
   tick rails, a triangular thumb, and a live value tooltip; the built
   control has only `accent-color` and no min/max/step readout
   (`INS:74-83`; `CSS:524-527`). This is the central tactile motif.
   Change: `studio/src/components/inspector/Inspector.tsx` +
   `studio/src/styles/studio.css`.
3. **Options cards do not read as a 2×2 drafted sheet** (`PARTIAL`, screen 1
   #9). Two columns with internal scroll; card 01 is stretched by the tall
   registry card and leaves a large blank area. All four cards are never
   visible together. Change: `studio/src/components/options/OptionsDrawer.tsx`
   (`174-226`) and `studio/src/styles/studio.css` (`1023-1031`, add
   `align-items: start` / cap or collapse the registry).
4. **Journal rows cannot support an audit read** (`PARTIAL` /
   `MISSING-BUILDABLE`, screen 3 #15–19). No block/sequence id, UTC label, author,
   approval stamp, copy-digest button, or newest-first order; digest chips
   appear only when a digest exists (`JRN:141-154`; `TYPES:33-39`). Sequence,
   UTC, older/newer ordering, and clipboard copy are frontend-only changes.
   Change: `studio/src/components/journal/JournalPanel.tsx` and
   `studio/src/types/studio.ts`.
5. **Title block is only half populated** (`PARTIAL`, screen 2 #17–25, screen
   5 #2, screen 6 #28). It has part id, drawing, scale, units, projection,
   sheet, tolerance, rev, author; it lacks drawn/checked-by, date, format,
   one-of-N, material, finish, and any real hash. Some can be honest `—`
   fields; material/finish/hash need backend data. Change:
   `studio/src/components/drafting/DraftingSheet.tsx` (`273-310`),
   `studio/src/components/shell/AppShell.tsx` (`81-96`), and the
   `TitleBlockData` contract.
6. **Drawer drafting furniture is thinner than the mock** (`PARTIAL`, screen
   1 #8, #10, #12, #52). Only one corner `+` per card, no card icons, no
   standards reference, and no full sheet-footer title block. Change:
   `studio/src/styles/studio.css` (`1033-1053`, add four corner marks via
   `::before`/`::after` or spans) and `OptionsDrawer.tsx:228-247`.
7. **Duplicate nested headings waste drawer space** (`PARTIAL`, screen 1 #13,
   #27). Card headers say “01 // Parametric Inspector…” and the embedded
   component repeats “INSPECTOR”; card 02 repeats “EXPORT & HANDOFF”. Change:
   pass a `compact`/`hideHeading` prop or remove the inner `<header>` in
   `studio/src/components/inspector/Inspector.tsx:112-117` and
   `studio/src/components/export/ExportPanel.tsx:135-137`.
8. **Spacing tokens are not in use** (`PARTIAL`, screen 7 #21–22). DESIGN.md
   specifies a 4 px rhythm and 12/16 px gutters; the CSS hardcodes 1–34 px
   values and has no `--space-*` custom properties. Change:
   `studio/src/styles/tokens.css` plus the four component sheets.
9. **`--radius: 2px` contradicts the zero-radius system** (`PARTIAL`, screen
   7 #23). `base.css` applies it to every button/input; most component
   styles override to 0, so the app is inconsistent. Change:
   `studio/src/styles/tokens.css:52` and `studio/src/styles/base.css:49-72`.
10. **Precision editing lacks min/max/step and steppers**
    (`MISSING-BUILDABLE`, screen 4 #9–10). The limits are already in
    `studio/src/types/studio.ts:55-66` and passed to each input
    (`INS:47-87`), so printing the range and adding ± buttons is frontend
    work. Change: `Inspector.tsx` + `studio.css`.
11. **Main-sheet section is a red plane, not a hatched cut face** (`PARTIAL`,
    screen 5 #14). AuxView has the 45° ANSI hatch, but the centerpiece sheet
    only shows the plane/flags. Change: `studio/src/viewer/SceneCanvas.tsx`
    (`780-820`) and `studio/src/styles/drafting.css`.
12. **Export receipt has no verification affordance** (`PARTIAL`, screen 1
    #32). The panel prints filename and real byte count, but no hash and no
    copy action; the journal digest chip is not clickable. Change:
    `studio/src/components/export/ExportPanel.tsx:189-193` and
    `JournalPanel.tsx:148-152` (copy button; a hash line can honestly say
    “not provided” until the backend returns one).

## Needs backend (grouped by capability)

1. **Studio audit / hash chain / sign-off.** A Studio route for the core
   `EventJournal.verify_chain` (`src/kala_agent/persistence/journal.py:220`),
   block numbers, authors, artifact SHA-256, policy/tool-class events,
   approvals, and HSM/pre-commit signing. The Studio server currently has no
   journal route (`src/kala_agent/studio/server.py:1-25`); `AppShell` passes
   `hashChainStatus="unknown"` (`APP:140`).
2. **FEA / analysis.** Stress, material yield, safety factor, mesh node
   counts, heatmaps, and report generation. No analysis route or result
   storage in the Studio API; the FE sheet and the hash-chain simulation
   card are placeholders.
3. **Extended geometry / material.** Wall thickness, bores, fillets, flange
   housings, PCD bolt patterns, custom scripts, material (e.g. Ti-6Al-4V),
   surface finish, general tolerance, and revision/date/checker metadata.
   The Studio primitive contract is only box/cylinder/sphere
   (`src/kala_agent/studio/schemas.py`; `TYPES:5-6, 77-97`).
4. **Constraint/topology solver.** DOF/equilibrium state, stiffness,
   governing rules, bolt preload, re-solve, and the stress-equilibrium
   terminal. No solver API exists in the Studio.
5. **Sheet / plotter / BOM.** Multi-sheet registry, A3/A4 format and 1-of-N
   metadata, PDF/DXF/FCStd writers, HPGL/vector spooler queue, and
   BOM/fastener records. Only STL/STEP export and neutral handoff exist
   (`CLIENT:198-318`; `SERVER:288-367`).
6. **Measurement / drawing tools.** Caliper span, datum pin, compass,
   T-square orientation lock, eraser shield, and cursor model-space
   telemetry. There is no measurement/picking API; the sheet palette only
   exposes reset, dims, section, and grid (`SHEET:188-238`).
7. **Provider / connection admin.** Per-provider ACTIVE/OFFLINE state,
   persisted remote-egress allowlist, bearer-token rotation, CLI bridge,
   and IPC latency/PID telemetry. The real settings route stores/clears keys
   only (`CLIENT:173-183`; `SERVER:230-272`); no rotation or telemetry.
8. **Structured agent output.** Inline agent result tables, material
   annotations, B-Rep status, and digest chips in chat. `ChatResponse`
   carries only `text/provider/model/offline` (`TYPES:202-207`); the chat
   currently shows real active-geometry context instead
   (`CHAT:288-307`).

## Honest omissions confirmed

No Stitch placeholder value was found in shipped source or in the P3
screenshots: `28.16`, `14208`, `8f3a`, `1.1.3`, `Llama`, `FIPS`, `keytar`,
`B-Rep Verified`, and the fabricated PIDs/latencies are absent. The only
`HASH-CHAIN OK` literal is the guarded `verified` branch in
`JRN:222`, unreachable because `APP:140` passes `unknown`; the neutral stamp
is `SESSION LOG`. The title block and bottom bar show real store/geometry
values or explicit `—` / `local est.` / `not provided` states.

## Method limits

- The audit is source-and-screenshot based; I did not launch the app. The P3
  shots at 1440×900 and 1280×800 were used to confirm the rendered layout,
  plus the P1/P2/P3 REPORTs for behavioral evidence.
- Some Stitch screens are mobile folio/audit concepts rather than the
  desktop 4-zone wave. Their sheet-management, plotter, FEA, and hardware
  sign-off elements are marked `N/A` only where the element is genuinely a
  different product surface or decoration; where a real feature is implied,
  it is marked `MISSING-NEEDS-BACKEND` instead.
- The Solid Edge image is suggestions only; it is not counted in the status
  totals and does not imply a defect against the wave brief.


<!-- END -->

