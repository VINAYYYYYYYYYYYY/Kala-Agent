# P1 shell — report

Status: **complete**. Vellum 4-zone shell, bottom ruler bar, Options drawer, store contract, theme system, and the three final slot stubs are implemented in `studio/`. No commit, push, deploy, credential read, or delegation was performed. No files outside `studio/**` were touched except this report.

## Gate results

Run from `/home/v9i/Projects/Kala-Agent/studio`:

| Command | Result |
| --- | --- |
| `npm run typecheck` | clean (exit 0) |
| `npm test` | **14 test files passed, 178 tests passed** (baseline 159; +19) |
| `npm run build` | success, 46 modules transformed; only the pre-existing >500 kB `three` chunk warning |

Visual smoke checks were also run with headless Chromium against the real Python studio backend at **1440×900** and **1280×800**, in both themes and with the Options drawer open:

- grid is exactly `left 340 / flex center / right 320` at 1440 (280/740/260 at 1280 after the compact breakpoint);
- bottom bar is 48 px and had **0 px horizontal overflow** at both sizes;
- exactly **one** `.viewer-canvas` was mounted with the drawer open and closed;
- theme switch applied `[data-theme="slate"]` and persisted only the theme name.

## Final stub contracts (P2 builds against these)

### `src/components/drafting/DraftingSheet.tsx`

```ts
/**
 * Real title-block values. Every field is derived from workspace state or
 * is an explicit empty state — no revision, tolerance, or verification is
 * invented.
 */
export interface TitleBlockData {
  sheet: string;
  partId: string;
  title: string;
  scale: string;
  units: string;
  projection: string;
  tolerance: string;
  revision: string;
  author: string;
}

/**
 * FINAL prop contract for the center drafting sheet.
 *
 * P2 owns the visual implementation; this P1 stub renders the existing
 * SceneCanvas inside a simple sheet frame. `viewResetToken` increments on
 * the bottom-bar Reset View action; remounting the canvas re-frames the
 * camera until P2 wires a direct camera reset.
 */
export interface DraftingSheetProps {
  /** What SceneCanvas needs: the current mesh and its provenance. */
  mesh: StudioLocalPreview | PreviewResponse | null;
  meshSource: 'local' | 'server' | 'none';
  showGrid: boolean;
  showAxes: boolean;
  showDimensions: boolean;
  material: MaterialKind;          // from src/state/store
  background: BackgroundKind;      // from src/state/store
  /** 2D pencil drafting mode or 3D clay solid mode. */
  renderMode: RenderMode;
  /** Whether the Section A-A cut plane is shown. */
  showSection: boolean;
  /** Real title-block labels for the active part. */
  titleBlock: TitleBlockData;
  /** Millimetres; the backend primitive contract is metric. */
  units: StudioUnits;
  /** Increments when the operator asks to re-frame the view. */
  viewResetToken: number;
}
```

`DraftingSheet` is the only owner of the app's single `SceneCanvas`.

### `src/components/drafting/AuxView.tsx`

```ts
/**
 * FINAL prop contract for the right-top auxiliary view.
 *
 * P2 owns the visual implementation; this P1 stub renders tab headers and
 * placeholder text so the shell is fully functional.
 */
export interface AuxViewProps {
  /** Active aux tab. */
  auxView: AuxViewKind;
  /** Called when the operator picks another tab. */
  onAuxViewChange(view: AuxViewKind): void;
  /** Active primitive kind. */
  primitive: PrimitiveKind;
  /** Active parameters, in millimetres. */
  parameters: StudioParameters;
  /** Current real mesh bounds (local or server), or null before a preview. */
  bounds: BBox | null;
}
```

Current tabs: `'section' | 'ortho' | 'tolerance'`. The stub shows real primitive/parameter/bounds facts and an explicit placeholder; it does not draw derived geometry.

### `src/components/journal/JournalPanel.tsx`

```ts
/**
 * FINAL prop contract for the right-hand journal slot.
 *
 * P2 owns the visual implementation; this P1 stub renders a plain list so
 * the shell is fully functional.
 */
export interface JournalPanelProps {
  /** Bounded journal rows from `useUiState(selectJournal)`. */
  entries: ReadonlyArray<JournalEntry>;
  /** Real loopback connection state, rendered in the ledger footer. */
  connectionStatus: ConnectionStatus; // from src/state/store
  /**
   * Hash-chain verification reported by a real backend. The shell passes
   * `'unknown'` until such a report exists; never derive "OK" from local
   * state.
   */
  hashChainStatus: HashChainStatus;   // 'unknown' | 'verified' | 'broken'
}
```

The shell currently passes `hashChainStatus="unknown"`; the rendered footer says `HASH-CHAIN —`, never a fake OK.

## Store contract

New shared types live in `src/types/studio.ts` and are re-exported from `src/state/store.ts`:

```ts
export type RenderMode = 'pencil2d' | 'clay3d';
export type AuxViewKind = 'section' | 'ortho' | 'tolerance';
export type ThemeName = 'vellum' | 'slate';
export type StudioUnits = 'mm';
export type OptionsCard = 'inspector' | 'export' | 'providers' | 'connection';
export type JournalKind = 'preview' | 'chat' | 'tool' | 'export' | 'connection' | 'system';
export type HashChainStatus = 'unknown' | 'verified' | 'broken';

export interface JournalEntry {
  id: string;
  ts: number;
  kind: JournalKind;
  message: string;
  digest?: string;
}

export const JOURNAL_CAP = 500;
```

`ConnectionStatus` (used by `JournalPanelProps`) remains:

```ts
export type ConnectionStatus =
  | { kind: 'idle' }
  | { kind: 'connecting' }
  | { kind: 'ready'; connection: StudioConnection }
  | { kind: 'error'; message: string };
```

### New `UiState` fields

```ts
renderMode: RenderMode;              // default 'pencil2d'
auxView: AuxViewKind;                // default 'section'
showSection: boolean;                // default true
optionsOpen: boolean;                // default false
optionsCard: OptionsCard;            // default 'inspector'
theme: ThemeName;                    // default readStoredTheme() → 'vellum'
journal: ReadonlyArray<JournalEntry>;// default []
viewResetToken: number;              // default 0; bottom-bar Reset View increments
connectionRetryToken: number;        // default 0; Options retry increments
```

`viewport.background` now defaults to `'paper'`. `AppShell` derives the canvas background from the theme (`paper` for vellum, `dark` for slate) so the drafting sheet matches the workspace; the store field remains available for P2's eventual explicit background control.

### New actions

```ts
| { type: 'renderMode/set'; mode: RenderMode }
| { type: 'auxView/set'; view: AuxViewKind }
| { type: 'section/toggle' }
| { type: 'section/set'; show: boolean }
| { type: 'options/open'; card?: OptionsCard }
| { type: 'options/close' }
| { type: 'options/card'; card: OptionsCard }
| { type: 'theme/set'; theme: ThemeName }
| { type: 'journal/append'; entry: JournalEntry }
| { type: 'journal/clear' }
| { type: 'view/reset' }
| { type: 'connection/retry' }
```

`journal/append` keeps at most `JOURNAL_CAP` (500) rows and drops the oldest first. The reducer stays pure; ids/timestamps are created by `createJournalEntry` before dispatch.

### Helpers and selectors

```ts
export function createJournalEntry(input: {
  kind: JournalKind;
  message: string;
  digest?: string;
  ts?: number;
  id?: string;
}): JournalEntry;

export function appendJournal(input: {
  kind: JournalKind;
  message: string;
  digest?: string;
  ts?: number;
  id?: string;
}): void;

export const THEME_STORAGE_KEY = 'kala-studio.theme';
export function readStoredTheme(): ThemeName;
export function persistTheme(theme: ThemeName): void;

export const selectConnection = (state: UiState) => state.connection;
export const selectPrimitive = (state: UiState) => state.primitive;
export const selectParameters = (state: UiState) => state.parameters;
export const selectPreview = (state: UiState) => state.preview;
export const selectViewport = (state: UiState) => state.viewport;
export const selectRenderMode = (state: UiState) => state.renderMode;
export const selectAuxView = (state: UiState) => state.auxView;
export const selectShowSection = (state: UiState) => state.showSection;
export const selectOptionsOpen = (state: UiState) => state.optionsOpen;
export const selectOptionsCard = (state: UiState) => state.optionsCard;
export const selectTheme = (state: UiState) => state.theme;
export const selectJournal = (state: UiState) => state.journal;
export const selectServer = (state: UiState) => state.server;
export const selectChat = (state: UiState) => state.chat;
export const selectViewResetToken = (state: UiState) => state.viewResetToken;
export const selectConnectionRetryToken = (state: UiState) => state.connectionRetryToken;
```

Only the theme name is persisted, in `localStorage['kala-studio.theme']`; no token, origin, key, chat text, or journal content is stored there.

### Journal appenders wired in P1

| Source | Kind | What is recorded |
| --- | --- | --- |
| `BootGate` bridge/manual success, failure, retry, registry load | `connection` / `system` | origin + mode (no token), error text, integration/provider counts |
| `previewController` request / success / timeout / failure | `tool` / `preview` | endpoint + primitive, engine + real vertex count, real error |
| `ChatPanel` send / response / failure | `chat` / `tool` | prompt/response preview, provider/model endpoint, real error |
| `ExportPanel` request / receipt / failure | `tool` / `export` | export/handoff endpoint + format, filename + real byte count, real error |
| Options retry | `connection` | "Reconnect / retry requested from Options" |

`JournalKind` also includes `'tool'` for real future agent tool-call events; the current backend has no agent tool-call protocol, so P1 records actual HTTP tool calls rather than inventing agent tool events.

## 4-zone layout and components

- `AppShell` now renders `BootGate → .studio-shell` with:
  - left `.studio-zone-left` → existing `ChatPanel`;
  - center `.studio-zone-center` → `DraftingSheet` (single `SceneCanvas`) + `BottomBar`;
  - right `.studio-zone-right` → `AuxView` (250 px) + `JournalPanel` (rest).
- No top header and no app footer. Connection info is in the journal footer and Options card 04.
- `BottomBar` (48 px): render-mode switch, Grid/Axes/Dims/Section A-A toggles, Reset View, live ENV/VOL/AREA readout with `local est.` / `server` source, Export STEP (opens drawer card 02), real FreeCAD registry status, ⚙ Options.
- `OptionsDrawer`: 4 cards (Inspector; ExportPanel + IntegrationsPanel; ProviderPanel; connection info + retry), theme switch, Esc closes, focus trapped, focus returned to trigger, `role="dialog"` + `aria-modal="true"`.
- `Inspector` gained the primitive selector (Box/Cylinder/Sphere) that previously lived in the retired toolbar, plus Vernier-style range sliders under each numeric input.
- `BootGate` connection screen is restyled as a drafted vellum card; manual browser connect still works exactly as before.
- Local fonts are bundled with `@fontsource-variable/space-grotesk`, `@fontsource-variable/newsreader` (incl. italic), and `@fontsource-variable/jetbrains-mono`; no remote font URL and no CSP change.

## Files changed

New:

- `studio/src/components/bottombar/BottomBar.tsx`
- `studio/src/components/options/OptionsDrawer.tsx`
- `studio/src/components/drafting/DraftingSheet.tsx`
- `studio/src/components/drafting/AuxView.tsx`
- `studio/src/components/journal/JournalPanel.tsx`
- `studio/src/styles/tokens.css`
- `studio/src/styles/base.css`
- `studio/src/styles/layout.css`

Modified:

- `studio/package.json`, `studio/package-lock.json` (font dependencies only)
- `studio/src/main.tsx` (font + split CSS imports, pre-paint theme)
- `studio/src/types/studio.ts`
- `studio/src/state/store.ts`
- `studio/src/state/previewController.ts`
- `studio/src/components/shell/AppShell.tsx`
- `studio/src/components/shell/BootGate.tsx`
- `studio/src/components/inspector/Inspector.tsx`
- `studio/src/components/export/ExportPanel.tsx`
- `studio/src/components/chat/ChatPanel.tsx`
- `studio/src/styles/studio.css` (rewritten to component styles only)
- `studio/test/AgentWorkspace.dom.test.tsx` (rewritten for the new layout)
- `studio/test/layoutCss.test.ts` (rewritten for the new layout/tokens)
- `studio/test/store.test.ts` (journal/shell-state coverage added)
- `studio/test/ExportPanel.dom.test.tsx` (export-receipt journal assertion added)

Deleted (old layout, functions moved):

- `studio/src/components/shell/Header.tsx`
- `studio/src/components/shell/RightRail.tsx`
- `studio/src/components/shell/workspaceView.ts`
- `studio/src/components/sidebar/Sidebar.tsx`
- `studio/src/components/toolbar/Toolbar.tsx`

Not touched: `studio/index.html` CSP, `studio/electron/**`, `studio/scripts/**`, `electron-builder.json`, backend/Python/Rust/solidedge files.

## Known gaps / intentionally stubbed

1. **P2 visual ownership is intact.** `DraftingSheet`, `AuxView`, and `JournalPanel` are functional but visually simple; their prop contracts above are frozen.
2. **Render mode and Section A-A are state-only in P1.** The switch and section flag are real store state and are passed to `DraftingSheet`; P2a still has to render pencil/clay and the section cut in the viewport. P1 shows the mode badge and red dashed cut line as a stub.
3. **Reset View remounts `SceneCanvas`** via `key={viewResetToken}` to re-frame the camera. P2a should replace this with a direct camera-reset API.
4. **AuxView has no derived section/ortho/tolerance geometry.** It shows real primitive/parameter/bounds facts plus explicit placeholders; no tolerance stack is computed or claimed.
5. **Hash-chain status is always `'unknown'`.** There is no backend hash-chain endpoint in this build, so the journal never renders `HASH-CHAIN OK`.
6. **FreeCAD pill is registry status only** (`available_via_neutral_file`, `native_api_unavailable`, `not_configured`, or `—`). It is not a live FreeCAD bridge/PID/latency claim.
7. **Options card 04 has no second token form.** Manual browser connect remains in `BootGate`; the drawer shows the live origin/mode/health/version and a real reconnect/retry action. No token is displayed or persisted.
8. **Bottom-bar source semantics:** ENV follows the server mesh bounds when a server preview exists; VOL/AREA are always closed-form nominal values from the active parameters. The badge says `server` when a server mesh is present and the tooltip explains the split; with only a local mesh it says `local est.`.
9. **Build warning:** the pre-existing `three` chunk remains >500 kB. No code-splitting change was made in P1.
