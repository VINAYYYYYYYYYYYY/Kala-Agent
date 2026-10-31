# Cell shell — REPORT (app-feel wave, 2026-10-31)

Worker: shell cell. Checkout: `/home/v9i/Projects/Kala-Agent/studio` (shared with
`composer` and `settings`). No commits, no stash, no reset; `studio/dist` was not
written (the visual build went to `/tmp/kala-shell-dist` and was served by
`tools/serve.sh` on port 8792).

## Result

The app now has one frame:

```
top bar 44px   [≡] KALA // STUDIO   [2D Pencil | 3D Clay]   Details  Export  ⚙
chat 360 (optional) | drafting sheet | Details 360 (optional)
status bar 28px (read-only): ● Connected · Browser fallback · ENV/VOL/AREA/SCALE/source · ● FreeCAD · backend v0.1.0
```

- Idle screen, 1440×900: **22 visible controls** (shot.mjs count). The wave
  diagnosis counted 36 on `before/01-main.png`.
- Details closed by default; opened on demand. No control has two homes.
- The render-mode pill, the bottom-bar mode/view/export/options buttons and the
  bottom-bar material control are gone. `src/styles/tokens.css` is untouched.

## Files changed (all owned by this cell)

New:
- `src/components/shell/TopBar.tsx` — banner, chat toggle, wordmark, the only
  render-mode control, Details, Export, Settings.
- `src/components/shell/DetailsPanel.tsx` — Properties / Views / Activity tabs,
  close button, Escape handling.
- `src/components/shell/ActiveGeometrySummary.tsx` — the DIMS / ENVELOPE /
  VOLUME / SOURCE table moved out of `ChatPanel` (copied from HEAD, same values
  and the same “measured by …” honesty).
- `src/styles/shell.css` — top bar, Details column and geometry summary.
- `test/TopBar.dom.test.tsx` (6 tests), `test/DetailsPanel.dom.test.tsx`
  (7 tests), `test/AppShellChrome.dom.test.tsx` (5 tests: layout + shortcuts).

Modified:
- `src/components/shell/AppShell.tsx` — top bar / `studio-main` grid / status bar;
  chat and Details are conditional grid columns; mounts `ExportDialog`; global
  Ctrl/Cmd+, and Ctrl/Cmd+B shortcuts (ignored inside text fields). The mesh
  memo, the single `designController`, title block and theme effect are
  unchanged.
- `src/components/bottombar/BottomBar.tsx` — read-only 28px status bar. Left:
  connection label (button → Settings › Connection, real state including
  Browser fallback). Centre: unchanged ENV/VOL/AREA/SCALE + source readout
  (incl. the provider readout helpers). Right: FreeCAD dot + backend version.
  Removed the props (`onOpenOptions` no longer exists).
- `src/components/drafting/DraftingSheet.tsx` — removed the render-mode pill;
  added the Axes toggle. The rail is now the only home of reset, dims, measure,
  section, grid, axes.
- `src/styles/layout.css` — new frame grid (top bar / chat | sheet | details /
  status bar), 360px chat and Details tracks (320px below 1400px), 28px status
  bar; removed the old bottom-bar mode/icon/export/options rules.
- `src/components/drafting/AuxView.tsx` — only the “new design gets the
  default XZ cut” effect moved out: it now lives in `AppShell`, which is
  always mounted, so it still runs while Details is closed and remounting the
  Views tab no longer resets a plane the operator chose. The view itself is
  unchanged.
- `src/styles/drafting.css` — removed the dead render-mode pill rules.
- Tests: `AppShellMeshStability` (journal assertion now reads the ledger in
  Details › Activity), `BottomBar` (rewritten for the status bar, 9 tests),
  `BottomBarDesignReadout`, `DraftingSheet` (no pill, six tools incl. Axes),
  `SurfaceMaterial` (one home: Details › Properties / Inspector),
  `previewStatus` (no BottomBar props), `designSystem` (two obsolete bottom-bar
  assertions replaced by the new truth: icon buttons on the sheet rail, no
  second material control, status bar 28px).

Untouched but re-run green: `MeasureTool`, `journalLedger`, `VernierSlider`
(their behaviour did not depend on the moved controls). `AuxView` keeps its
tests green after the effect move above.

`src/components/export/ExportDialog.tsx` already existed when AppShell was
wired (created by the settings cell), so **no placeholder was needed or
written**. `src/state/store.ts` and `src/types/studio.ts` were not edited.

## What moved where

| control | before | now |
|---|---|---|
| 2D Pencil / 3D Clay | sheet pill + bottom bar | top bar only |
| reset, dims, measure, section, grid | sheet rail + bottom bar | sheet rail only |
| axes | bottom bar | sheet rail only |
| Export | bottom bar + Options card 02 | top bar → Export dialog |
| Settings (Options) | bottom bar ⚙ Options | top bar ⚙ / Ctrl/Cmd+, |
| chat visibility | always on | top bar ≡ / Ctrl/Cmd+B, column removed when closed |
| Active geometry table | chat intro | Details › Properties |
| primitive sliders + surface material | Options card 01 + bottom bar | Details › Properties (no design) |
| AuxView | always-on right column | Details › Views |
| Journal | always-on right column | Details › Activity |
| connection | journal footer + Options card 04 | status bar label → Settings › Connection |
| backend version | journal footer | status bar right |

## Tests (what I actually ran)

`npx tsc -p tsconfig.json --noEmit` → no errors (whole project, at the time of
running).

`npx vitest run <my 14 files>` → **14 files, 144 tests, all passing**:

| file | tests |
|---|---|
| `test/TopBar.dom.test.tsx` | 6 |
| `test/DetailsPanel.dom.test.tsx` | 7 |
| `test/AppShellChrome.dom.test.tsx` | 5 |
| `test/AppShellMeshStability.dom.test.tsx` | 4 |
| `test/BottomBar.dom.test.tsx` | 9 |
| `test/BottomBarDesignReadout.dom.test.tsx` | 8 |
| `test/DraftingSheet.dom.test.tsx` | 21 |
| `test/MeasureTool.dom.test.tsx` | 6 |
| `test/AuxView.dom.test.tsx` | 24 |
| `test/journalLedger.dom.test.tsx` | 24 |
| `test/previewStatus.dom.test.tsx` | 6 |
| `test/SurfaceMaterial.dom.test.tsx` | 4 |
| `test/VernierSlider.dom.test.tsx` | 9 |
| `test/designSystem.test.ts` | 11 |

Whole suite at hand-back (other cells were still landing): 3 files fail —
`AgentWorkspace` (location assertions: `.bottom-bar-options`, render-mode in the
bottom bar, always-on journal/aux column), `ManualConnect` (asserts
`.journal-panel-footer` is always present; the journal now lives in Details ›
Activity), `layoutCss` (pins the old four-zone grid, 48px bottom bar,
`--zone-left-width`, and an options selector that no longer exists). These are
the integration-step tests named in WAVE.md; no behaviour assertion was deleted
to make them pass.

## Screenshots (1440×900, cells/shell/shots/)

- `idle-1440.png` — idle, Details closed, 22 visible controls.
- `details-1440.png` — Details open on Properties (geometry summary +
  primitive inspector).
- `slate-1440.png` — slate theme, Details closed.
- `details-views-1440.png` — extra: Details › Views (AuxView filling the column).
- `details-activity-1440.png` — extra: Details › Activity (journal filling the
  column).

Build used: `npx vite build --outDir /tmp/kala-shell-dist --emptyOutDir`,
served with `tools/serve.sh 8792 /tmp/kala-shell-dist`, screenshots with
`tools/shot.mjs`. Server stopped and temp dirs removed after the run.

## Left over / notes for integration

- `test/layoutCss.test.ts` still asserts the old four-zone grid and 48px bar;
  update it to `.studio-shell` rows `auto minmax(0,1fr) auto`, `.studio-main`
  tracks, 28px status bar, and drop the `.options-card-body > .export-panel`
  selector.
- `AgentWorkspace` and `ManualConnect` need their location assertions moved to
  the top bar / status bar / Details panel (behaviour unchanged).
- At ≤1400px the chat and Details tracks are 320px each; there is no overlay or
  stacking behaviour below 900px beyond narrower tracks — the app is
  desktop-first at 1440.
- The Details tab strip has `role="tablist"` / `role="tab"` /
  `role="tabpanel"` with roving `tabIndex`, but arrow-key navigation between
  tabs is not implemented (AuxView keeps its own arrow keys). Not required by
  the task; noted in case the integration pass wants parity.
