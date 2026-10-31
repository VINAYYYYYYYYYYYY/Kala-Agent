# Cell settings — report

## What this cell did

Replaced the four-card "Options & Engineering Drawer" with a focused
**Settings** dialog and moved the export card into its own **Export** dialog.
The visual language is unchanged: same tokens, fonts, inked paper, hairline
rules, mono caps labels, Newsreader italic notes, zero radii. `tokens.css`,
`src/state/store.ts` and `src/types/studio.ts` were not touched.

## Files changed

| File | Change |
|---|---|
| `studio/src/components/options/OptionsDrawer.tsx` | Rewritten as the Settings dialog. Same name, file and props (`open`, `card`, `onCardChange`, `onClose`). |
| `studio/src/components/export/ExportDialog.tsx` | **New.** `ExportDialog({ open, onClose })`, titled `Export`, renders `<ExportPanel showHeading={false} />`. |
| `studio/src/components/export/ExportPanel.tsx` | With an active design the primitive controls are wrapped in a closed `<details>` disclosure titled `Export the primitive instead`. With no design the primitive block renders directly. Export logic, journal entries and refusals untouched. |
| `studio/src/styles/settings.css` | **New rules.** Shared `app-modal-backdrop`; Settings dialog, header, left nav, sections, rows, theme switch, honesty line, CAD blocks and connection facts; Export dialog shell and the disclosure. |
| `studio/src/styles/studio.css` | Deleted all now-dead `options-*` rules (the 2×2 grid, cards, ticks, drawer nav, theme switch, connection facts, title-block strip and the old options media queries) and the `.options-card` / `.options-connection` entries from the shared single-column selector list. Kept the `export-panel*`, `provider-key*`, `panel-list*`, `connection-form*` and `integration*` rules. |
| `studio/test/OptionsDrawer.dom.test.tsx` | Rewritten for the Settings sections. |
| `studio/test/ExportDialog.dom.test.tsx` | **New.** 7 tests: closed/open modal, primitive-only, design-first + disclosure, no registry, Esc/close, focus return. |
| `studio/test/designExport.dom.test.tsx` | Added a location test: the design block precedes the primitive disclosure (behaviour assertions unchanged). |
| `studio/test/designSystem.test.ts` | Replaced the removed Options-grid assertions (`.options-drawer-grid`, `.options-card`, ticks) with Settings-dialog assertions (860×600 sizing, fixed nav column, active nav marker). The two bottom-bar assertions in this file still fail and are shell-owned. |
| `studio/test/ProviderPanel.dom.test.tsx` | Header comment only (Options card 03 → Settings › Models & keys). No test or behaviour change; `compact` already existed and every behaviour assertion stayed. |

## What moved where

| Old Options card / control | New home |
|---|---|
| Card 01 — Parametric inspector & primitive controls | Details › Properties when no design is active (shell cell; removed here). |
| Card 02 — Neutral-file export & handoff + integration registry | Export dialog (design first; primitive behind the disclosure). Registry → Settings › CAD. |
| Theme switch in the drawer header | Settings › Workspace. |
| Card 03 — Model gateway & API providers | Settings › Models & keys (`ProviderPanel compact` + the “keys are sent once … never echoed” line). |
| Card 04 — Loopback boot gate & recovery | Settings › Connection (same facts, retry button and token notes). |
| Integration registry | Settings › CAD, above the live CAD provider rows. |
| Footer title block, decorative subtitle, “Dock to Scale Ruler” wording | Removed. |

The Settings dialog is 860×600 (never taller than the viewport), has a
`role="tablist"` left nav (`Workspace`, `Models & keys`, `CAD`, `Connection`,
ids `workspace` / `providers` / `cad` / `connection`), one `role="tabpanel"`
at a time, a Newsreader-italic sentence per section, `role="dialog"` +
`aria-modal`, Esc closes it, `aria-label="Close settings"` on the close
button, focus is trapped and returned to the opener.

Settings › CAD renders `selectCadProviders` read-only (name, version,
available/unavailable reason) and runs the same read-only provider probe as
the composer **only when the store has not been probed yet** (`status ===
'idle'`), so the section is useful on first open. The CAD pick itself stays
in the composer.

## Verification actually run

All shells used a throwaway HOME/data dir per WAVE.md.

- `npx tsc -p tsconfig.json --noEmit` → **0 errors**.
- `npx vitest run test/OptionsDrawer.dom.test.tsx test/ExportDialog.dom.test.tsx test/ExportPanel.dom.test.tsx test/designExport.dom.test.tsx test/ProviderPanel.dom.test.tsx`
  → **5 files, 46 tests passed**:
  - OptionsDrawer 15
  - ExportDialog 7
  - ExportPanel 8
  - designExport 12
  - ProviderPanel 4
- `npx vitest run test/designSystem.test.ts` → **11/11 pass**, including the 3 new Settings-dialog assertions and the shell's updated bottom-bar assertions.
- Full `npx vitest run` at 16:00 → **814 passed / 843 tests; 3 files failed, 29 tests**, all in whole-app tests left for the integration step:
  - `AgentWorkspace.dom.test.tsx` (20) — still opens `.options-card-*` / `.options-drawer-close` and asserts the old drawer, plus old chrome expectations.
  - `layoutCss.test.ts` (7) — old 4-zone/bottom-bar/options-grid expectations; `layout.css` was already rewritten by the shell cell.
  - `ManualConnect.dom.test.tsx` (2) — app-shell boot path.

  No failure is in a file this cell owns or in a test this cell owns.

## Screenshots

Built with `npx vite build --outDir /tmp/kala-settings-dist --emptyOutDir`,
served with `tools/serve.sh 8794 /tmp/kala-settings-dist`, captured at
1440×900 with `tools/shot.mjs`.

| File | Shows |
|---|---|
| `shots/settings-providers-1440.png` | Settings on **Models & keys** (required shot). |
| `shots/export-1440.png` | Export with an active design from a Local helper + OpenCascade turn (required shot). |
| `shots/export-primitive-open-1440.png` | Same Export dialog with `Export the primitive instead` expanded, proving the disclosure. |
| `shots/settings-workspace-1440.png` | Settings › Workspace theme switch. |
| `shots/settings-cad-1440.png` | Settings › CAD registry. |
| `shots/settings-connection-1440.png` | Settings › Connection facts and retry. |

The real design turn used the local deterministic helper and OpenCascade
(5/5 checks, design `6cdc6064`); the screenshot shows the design-first panel
with SCRIPT/STEP downloads and `EXPORT DESIGN STEP`.

## Left over / notes for integration

- `AgentWorkspace`, `layoutCss` and `ManualConnect` still target the old
  Options drawer or the old layout contract. They are whole-app tests and
  were deliberately left for the integration step, as WAVE.md says.
- The shell cell's `AppShell` already mounts this `ExportDialog` and
  `OptionsDrawer`; no app-shell change was made by this cell.
- Settings › CAD may issue one provider probe on first open when the composer
  has not probed yet; both paths write the same store fields
  (`server/cad-providers*`), and the composer remains the only place that
  selects a CAD target.
- The screenshots include the other cells' in-progress chrome around the
  dialogs; the dialogs themselves are this cell's build.

## Cleanup

Stopped the 8794 server and removed this cell's temp dirs/build output
(`/tmp/kala-settings-*`, `/tmp/kala-look-8794-*`, the ad-hoc screenshot
scripts/images). `studio/dist` was never written.
