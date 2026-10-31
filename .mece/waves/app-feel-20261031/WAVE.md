# Wave: make Kala Studio feel like an app, not a dashboard (2026-10-31)

## User request (verbatim)
"check the ui clean out the options, currently it feels like its dashbord rather than an app so can you make it an app rather than looking it like an dashbord ever option that is been given stright away make it feel like an app, but keep the overall design laguage the same"

**History, read it.** The 2026-10-21 AS 1100 redesign (`../as1100-sheet-20261021/`) was REVERTED by the user. It invented a new visual language and new features. This wave is the opposite. Keep the look exactly as it is (same tokens, fonts, paper, ink, vermilion, drafting frame, title block, mono labels, Newsreader italic help text, vellum and slate themes). Change only **what is visible when, and where each control lives**. Add no new product features and no new visual motifs.

## Diagnosis (screenshots in `before/`, 1440×900)
`before/01-main.png` (idle), `05-design.png` (after a design turn), `02-options.png` (Options), `03-apikeys.png`, `06-export.png`.
- **Every surface at once.** There are three fixed columns (chat 340, sheet, aux+journal 320), a bottom bar, and a 4-card "Options & Engineering Drawer". The idle screen has 36 visible controls.
- **Duplicates.**
  - 2D/3D appears twice: the sheet pill and the bottom bar.
  - Reset, dims, section and grid appear twice: the sheet tool rail and the bottom bar.
  - Export appears in the bottom bar and in Options card 02.
  - API keys appear in the chat header drawer and in Options card 03.
  - Theme is in the Options header.
- **Setup shown as content.** The chat column always shows starter presets, an "Active geometry" table, Provider, Model search (free-only checkbox, Reload), CAD target radios, a "ROUTE …" line and help text, all above the prompt.
- **Developer data always on.** The right column always shows the hash-chained Agent & Kernel journal (SEQ/UTC/DIGEST/CHAIN) and the aux view.
- **Dashboard modal.** Options is a 4-card grid. It mixes primitive sliders, export, API keys and loopback recovery on one screen.

## Target (binding)

```
┌ top bar 44px ──────────────────────────────────────────────────────────────────────┐
│ [≡] KALA // STUDIO                    [2D Pencil | 3D Clay]       Details  Export  ⚙ │
├ chat (360, collapsible) ─┬ drafting sheet = the app (unchanged look) ───────────────┤ ┌ Details (closed by default, 360) ┐
│ Agent            New chat│  design tabs (as today)        tool rail (single copy):  │ │ Properties | Views | Activity     │
│                          │  reset · dims · measure · section · grid · axes          │ │ Properties: active geometry +     │
│ (empty) 4 starter chips  │                                                          │ │   primitive inspector (no design) │
│ thread …                 │                                        title block       │ │ Views: AuxView (Section/Ortho/Tol)│
│ ┌──────────────────────┐ │                                                          │ │ Activity: JournalPanel            │
│ │ prompt               │ │                                                          │ └───────────────────────────────────┘
│ │ [Local helper ▾][CAD: OpenCascade ▾]   [Draft ↵] │                                │
├ status bar 28px (read-only): ● Connected · ENV … · VOL … · AREA … · SCALE … · source · ● FreeCAD · v0.1.0 ┤
```

### Decisions
- **D1 Same design language.**
  - Do not edit `src/styles/tokens.css`. Do not add colours, fonts, radii, shadows or font sizes. Use the existing custom properties. `test/designSystem.test.ts` must stay green: no raw px font-size, nothing under 9 px, radius 0.
  - Reuse the existing classes and patterns (mono caps labels, inked boxes, vermilion attention, stamp badges such as `CONFIGURED`).
- **D2 The sheet is the hero.** Only four things are permanent: the top bar, the chat sidebar (collapsible), the sheet and the status bar. Everything else opens on demand.
- **D3 One home per control. Nothing is shown twice.**
  - Render mode (2D Pencil / 3D Clay): top bar only.
  - View tools (reset, dims, measure, section, grid, axes): the sheet tool rail only.
  - Export: top bar → Export dialog.
  - Settings (the old "Options"): top bar ⚙ → Settings dialog.
  - Theme: Settings › Workspace.
  - API keys: Settings › Models & keys. Remove the chat header "API Keys" button and the inline key drawer.
  - Integration registry and CAD provider status: Settings › CAD.
  - Primitive sliders and surface material (old Options card 01): Details › Properties when no design is active.
- **D4 Progressive disclosure.**
  - Starter prompts appear only while the thread is empty (as chips in the thread area), and come back after "New chat".
  - The Active geometry table leaves the chat and moves to Details › Properties.
  - Provider, model and CAD target collapse into one row of two chip buttons under the prompt. Each opens a popover holding the existing pickers. The ROUTE line and the help paragraph go.
  - The design result card in the thread shows a one-line summary (checks passed/failed, measured by …) and expands on click.
- **D5 Status bar.** It replaces the BottomBar buttons and is read-only: connection, readout, source, FreeCAD status and backend version. Clicking the connection label opens Settings › Connection.
- **D6 Settings dialog.** It replaces the 4-card grid. It is a centred modal with a left nav and one section visible at a time: Workspace, Models & keys, CAD, Connection. The title is plain "Settings". Esc closes it.
- **D7 Export dialog.** It is focused. With an active design it shows that design's export (format, Export, script/STEP downloads). Without one it shows the primitive export (target + format), as today.
- **D8 Nothing is lost.** Every capability is reachable in ≤ 2 clicks. Keep the no-fake-data invariant: every value still comes from the store or the backend. Keep the a11y patterns: `role="dialog"` with `aria-modal`, Esc closes, labelled buttons.
- **D9 Tests follow the move.** Behaviour assertions stay, and location assertions change. Do not delete a behaviour test to make the suite pass.

## Shared contract (already in the tree; Opus wrote it, do not change it)
- **`src/types/studio.ts`:** `OptionsCard = 'workspace' | 'providers' | 'cad' | 'connection'` (the Settings sections) and `DetailsTab = 'properties' | 'views' | 'activity'`.
- **`src/state/store.ts` state:** `chatOpen` (true), `detailsOpen` (false), `detailsTab` ('properties') and `exportOpen` (false).
- **`src/state/store.ts` actions:**
  - `chrome/chat-toggle`;
  - `chrome/details-open {tab?}`, `chrome/details-close` and `chrome/details-toggle`;
  - `chrome/export-open` and `chrome/export-close`;
  - the existing `options/open {card?}`, `options/close` and `options/card`.
- **`src/state/store.ts` selectors:** `selectChatOpen`, `selectDetailsOpen`, `selectDetailsTab` and `selectExportOpen`.
- **New CSS files** (imported in `main.tsx`): `styles/shell.css` (cell shell), `styles/composer.css` (cell composer) and `styles/settings.css` (cell settings).
- **Component contract between cells:**
  - `OptionsDrawer` keeps its name, file and props (`open`, `card`, `onCardChange`, `onClose`); it now renders the Settings dialog.
  - Cell settings adds `src/components/export/ExportDialog.tsx`, exporting `ExportDialog(props: { open: boolean; onClose(): void })`.
  - Cell shell mounts both from `AppShell`.

## Cells (run in parallel on one shared checkout; stay inside your files)
| cell | owns |
|---|---|
| `shell` | `components/shell/AppShell.tsx`, new `shell/TopBar.tsx`, new `shell/DetailsPanel.tsx`, `bottombar/BottomBar.tsx`, `drafting/DraftingSheet.tsx`, `drafting/AuxView.tsx`, `journal/*`, `inspector/*`; CSS `layout.css`, `drafting.css`, `journal.css`, `bottombar.css`, `shell.css` and, in `studio.css`, only `inspector-*` / `viewer-canvas*` / `surface-material*`; their tests |
| `composer` | `components/chat/*`; CSS `chat.css`, `models.css`, `composer.css` and the `chat-panel*` rules in `studio.css`; the chat/provider/model/CAD-target/design-card tests |
| `settings` | `components/options/*`, `components/export/*`, `components/panels/*`; CSS `settings.css` and, in `studio.css`, the `options-*`, `export-panel*`, `provider-key*`, `panel-list*`, `connection-form*` and `integration*` rules; the OptionsDrawer/ExportPanel/designExport/ProviderPanel tests |

`AgentWorkspace`, `AppBoot`, `AppJournalPollerGate`, `ManualConnect`, `layoutCss` and `designSystem` are whole-app tests. Leave them for the integration step unless they test only your files. While the other cells work, typecheck and test failures in files you do not own are their work in progress. Do not "fix" them.

## Rules
- **USER DATA.** In every shell: `export RUSTUP_HOME=/home/v9i/.rustup CARGO_HOME=/home/v9i/.cargo npm_config_cache=/home/v9i/.npm`, then `T=$(mktemp -d)` and `export HOME=$T KALA_AGENT_DATA_DIR=$T/ka KALA_STUDIO_DATA_DIR=$T/ks XDG_DATA_HOME=$T/xdg`. Never touch `/home/v9i/.local/share/kala-agent/**` or `~/Applications/**`.
- Never read credential, auth or settings files. Never run the real `claude`, `codex` or `opencode` CLIs.
- **Never write `studio/dist`** (it is shared). For a visual check, run `npx vite build --outDir /tmp/kala-<cell>-dist --emptyOutDir`, then `tools/serve.sh <your port> /tmp/kala-<cell>-dist`, then `node tools/shot.mjs <port> <png>`. Ports: shell 8792, composer 8793, settings 8794. Stop your server and `rm -rf` your temp dirs at the end.
- **`/tmp` quota is 7.7 GB.** Remove only dirs you created. Never touch `/tmp/opencode` or `/tmp/pytest-of-v9i`.
- No git commits, no stash, no reset, no checkout of files you do not own. No new npm dependencies. No backend (Rust/Python) changes.

## Gates (each cell, on its own files)
- `npx tsc -p tsconfig.json --noEmit` shows no errors in your files.
- `npx vitest run <your tests>` is all green.
- At least one screenshot at 1440×900 in `cells/<cell>/shots/`.
- `cells/<cell>/REPORT.md`: files changed, what moved where, test counts, screenshots and anything left over.

## Addendum, 2026-10-31: the user's direction changes the frame (supersedes D2)
The user said: "let the chat section stay at the center", and "rendering is not supposed to be the main thing, the important thing is the chat interaction; the render is supposed to be after the design is done in a CAD tool".
- **New D2.** The chat is the centred, primary surface. The drawing (DraftingSheet) is a secondary split panel. It is closed by default, opens automatically when a CAD-built design arrives, and can be opened from the top bar.
- Implemented by cell `chat-first` (`cells/chat-first/TASK.md`), after cell `integrate`.
