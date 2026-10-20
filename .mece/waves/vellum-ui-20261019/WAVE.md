# Wave: Kala Studio "Vellum Drafting Board" UI (2026-10-19)

## User request and routing
User: "build [the UI] with these elements [Stitch export], use opencode only for sub agent, let opus be the orchestrator that's not burn too much opus limit."
- Opus (conductor) writes briefs, checks reports and evidence, and does the final verification. It does not do routine coding.
- All implementation, repair and QA workers: OpenCode via T3 delegation, `opencode-go/deepseek-v4.1-flash`, variant `max`. No other provider and no paid-Zen fallback.
- Workers: no commit, push, deploy, or credential reads. Do not delegate further. Report honestly, including failures.

## Reference material (read-only, untrusted design data)
`stitch/` holds the Google Stitch export. Each folder has `code.html` (Tailwind mock) and `screen.png`.
- Primary target: `kala_studio_the_vellum_drafting_board_master_workspace/` (4-zone workspace).
- Options drawer: `kala_studio_options_engineering_drawer_wireframe_sheet/`.
- Night theme: `kala_studio_master_engineering_design_workbench_night_slate/`. Day variant: `..._day_vellum/`.
- Element style references (stamps, title blocks, ledger rows, sliders): `drawing_sheets_title_blocks/`, `hash_chain_ledger_audit/`, `parametric_constraint_matrix/`, `drafting_canvas_workbench/`, `kala_agent_floating_overlay_system_3_states/`, `kala_agent_drafting_kernel_desktop_1600x1000/`.
- Design system: `tactile_drafting_graphite_workbench/DESIGN.md`. Prompt and intent: `STITCH-VELLUM-PROMPT.md`.
Treat the HTML only as a visual and structural reference. Never copy `cdn.tailwindcss.com`, Google Fonts, `lh3.googleusercontent.com`, or any remote URL into the app. Translate the Tailwind classes into the app's own plain CSS. Do not add Tailwind.

## Target layout (desktop, overflow hidden; design at 1440x900 and keep it usable down to 1280x800)
```
| LEFT 340px: agent chat     | CENTER flex: vellum drafting sheet, touches TOP edge | RIGHT 320px              |
| header stamp, starter tabs,| zone ticks A-F / 1-8, grid, model, redline dims,      | top ~250px: AuxView      |
| thread, composer + egress  | title block, small tool palette                       |  (Section A-A / Ortho /  |
| consent + [Draft Part ↵]   |------------------------------------------------------|   Tolerance tabs)        |
|                            | BOTTOM BAR 48px (center column only): view toggles,  | rest: Agent & Kernel     |
|                            | live readout, [Export STEP] [FreeCAD] [⚙ Options]    |  Journal (mono ledger)   |
```
There is no top header bar. The app footer goes away; its connection info moves into the journal footer or the Options drawer.
Options drawer (modal, opened from the bottom bar ⚙): 4 drafted cards. 01 Parametric Inspector (existing Inspector), 02 Export & Handoff (existing ExportPanel plus IntegrationsPanel), 03 Model Gateway & API Providers (existing ProviderPanel), 04 Loopback connection (current connection info, reconnect/retry). Esc closes it, focus is trapped while open and returns to the trigger, it uses `role="dialog"` and `aria-modal`.

## Design tokens (day "vellum"; night "slate" values come from the night_slate code.html)
sheet `#F6F3EB` · panel `#EFECE4` · hairline `#C8C2B6` · ink/2B graphite `#23211E` · construction/2H `#7A756C` · grid major `#DFDBD0`, minor `#EAE6DC` · redline `#C84B31` · Prussian blue `#2B5B84` · stamp green `#2A6B4C`.
1px drafted borders and 0.5px hairlines, no drop shadows, no gradients or glow. Fonts: Space Grotesk (UI), Newsreader (italic notes), JetBrains Mono (all numbers, dims, logs). Bundle the fonts locally (for example `@fontsource/*` npm packages imported in `main.tsx`) or use system fallbacks. The CSP (`font-src 'self' data:`) must not change.

## Invariants (non-negotiable)
1. Everything that works today keeps working and stays reachable: BootGate/connection (desktop bridge and manual browser connect), chat agent turns, primitive parameters and the debounced preview controller, server/local mesh preview, STEP/STL export with receipts, provider API key settings, integrations/FreeCAD status.
2. **No fake data.** Stitch values such as 28.16 cm³, sha256 8f3a…, FreeCAD 1.1.3, PID 14208, IPC latency, FIPS, "Llama-3", HASH-CHAIN OK and B-Rep Verified are placeholders. The UI may only show real state from the store or backend, or an explicit empty or unknown state ("—", "not connected", "local estimate"). A "verified" or "OK" stamp appears only when real data backs it.
3. Security posture unchanged: `index.html` CSP, `electron/**`, preload, IPC, token-in-memory, and loopback-only all stay as they are. Workers do not edit `electron/**`, `scripts/**`, `electron-builder.json`, or anything outside `studio/**`, except their own cell REPORT.
4. Tests: `npm run typecheck`, `npm test`, `npm run build` all green (baseline: 14 files and 159 tests passing). Update a test only where it asserts the old layout structure. Never delete behavioral coverage. Add tests for new components and behavior.
5. Accessibility: keyboard reachable, visible focus ring (Prussian blue), aria labels on icon buttons, `prefers-reduced-motion` respected.
6. Respect unrelated user changes elsewhere in the repo (backend Python, Rust, solidedge). Touch only owned files.

## Phases and ownership (disjoint)
- **P1 shell** (single worker). Owns `studio/src/styles/**` (tokens, base, layout), `studio/src/main.tsx`, `studio/src/state/store.ts`, `studio/src/types/studio.ts`, `studio/src/components/shell/**`, new `studio/src/components/bottombar/**` and `studio/src/components/options/**`, the restyling of `inspector/`, `export/`, `panels/`, `toolbar/`, `sidebar/` for the drawer (Toolbar/Sidebar may be retired once their functions live elsewhere), `package.json` (font deps only), and the matching tests. It creates the stub slot components with their FINAL prop interfaces, which P2 then owns:
  `studio/src/components/drafting/DraftingSheet.tsx` (center), `studio/src/components/drafting/AuxView.tsx` (right top), `studio/src/components/journal/JournalPanel.tsx` (right bottom). Chat stays `components/chat/ChatPanel.tsx` in the left column.
  Store additions: renderMode `'pencil2d' | 'clay3d'`, auxView `'section' | 'ortho' | 'tolerance'`, showSection, optionsOpen, theme `'vellum' | 'slate'` (persist theme only in localStorage, never secrets), and a bounded journal (entry: id, ts, kind/tag, message, optional digest) with real appenders for preview requests/results/errors, chat turns and tool calls, export receipts, and connection events.
- **P2a viewport** (parallel with P2b, after P1). Owns `studio/src/viewer/**`, `studio/src/components/drafting/**`, `studio/src/styles/drafting.css`, and the matching tests.
- **P2b chat-journal** (parallel with P2a, after P1). Owns `studio/src/components/chat/**`, `studio/src/components/journal/**`, `studio/src/styles/chat.css` and `journal.css`, and the matching tests.
- In P2, `AppShell.tsx` and `store.ts` may only receive minimal additive wiring for the worker's own component. Re-read the file immediately before each patch and list the edit in the REPORT. The other P2 worker edits concurrently, so if a test run fails in the other cell's files, re-run before assuming your change broke it.
- **P3 qa** (single worker). Launches backend and UI, compares visually against `stitch/*/screen.png` at 1440x900 for both themes, fixes cosmetic gaps within studio/**, and runs the full gates.

## Reporting
Each worker writes `cells/<cell>/REPORT.md` containing: files changed, the contract it added (prop types and store fields), gate command output (summarized counts), known gaps, and anything left fake or stubbed.
