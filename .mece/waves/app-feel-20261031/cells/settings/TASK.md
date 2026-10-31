# Cell settings: Settings dialog (replaces the 4-card Options drawer) and a focused Export dialog

Read `../../WAVE.md` first. It holds the decisions, the contract and the rules. Look at `../../before/02-options.png` and `06-export.png`. They show the dashboard modal you are replacing.

## Build
1. **Settings dialog: `components/options/OptionsDrawer.tsx`.** Keep the name, the file and the props (`open`, `card: OptionsCard`, `onCardChange`, `onClose`).
   - A centred modal (`role="dialog"`, `aria-modal`, `aria-labelledby` pointing at the title `Settings`), about 860×600 and never taller than the viewport, using the same inked-paper modal look as today.
   - **Left nav** (`role="tablist"` or a nav of buttons with `aria-current`) with sections: `Workspace`, `Models & keys`, `CAD`, `Connection` (ids `workspace`, `providers`, `cad`, `connection`).
   - **Right:** one section at a time, under a short heading and one Newsreader-italic sentence.
   - **Section contents:**
     - `workspace`: the theme switch (Vellum / Slate), moved from today's drawer header.
     - `providers`: `<ProviderPanel compact />`, the API keys and provider status. Keep the "keys are sent once… never echoed" honesty line.
     - `cad`: the integration registry (`<IntegrationsPanel compact />`) and the live CAD provider rows from the store (`selectCadProviders`: name, version, available/unavailable reason). Read-only; the pick stays in the composer.
     - `connection`: today's card 04, unchanged (status, origin, backend, health, token-in-memory note, `Reconnect / Retry bridge`).
   - **Remove:** the primitive inspector card (it moved to Details › Properties), the export card (it moved to the Export dialog), the decorative subtitle, the title-block footer strip and the "Dock to Scale Ruler" wording.
   - Esc and the close button (`aria-label="Close settings"`) close it, and focus returns to the opener.
2. **Export dialog: new `components/export/ExportDialog.tsx`, exporting `ExportDialog(props: { open: boolean; onClose(): void })`.**
   - A focused modal titled `Export` (`role="dialog"`, `aria-modal`). It renders `<ExportPanel showHeading={false} />`.
   - In `ExportPanel`, with an active design, the design block comes first and is the primary action (format + `Export design STEP`, plus the SCRIPT/STEP downloads). The primitive "Direct export / handoff target" block sits below it, behind a small disclosure `Export the primitive instead`.
   - With no active design, show the primitive block directly.
   - The integration registry no longer appears here (it is in Settings › CAD).
   - Esc closes it. All existing export behaviour, journal entries and refusals stay byte-for-byte.
   - Cell shell may already have written a placeholder `ExportDialog.tsx`; overwrite it.
3. **Styles.** New rules go in `styles/settings.css`. Delete the dead `options-card*` grid and `options-drawer-title-block*` rules from `studio.css`. Same tokens and look.

## Tests you own
- Rewrite `OptionsDrawer.dom.test.tsx` for the sections: each nav item shows its section; theme switch; reconnect dispatches `connection/retry`; Esc closes; no inspector or export inside.
- Update `ExportPanel`, `designExport` and `ProviderPanel` as needed; keep every behaviour assertion.
- Add `test/ExportDialog.dom.test.tsx` covering: open/closed rendering, Esc/close calls `onClose`, design-first ordering when a design is active, and the primitive block when none is active.

## Done when
- No type errors in your files, and your tests are green.
- `shots/settings-providers-1440.png`: Settings on Models & keys. Open it with STEPS `role=button|Settings`; if the top bar is not there yet, use `role=button|Options`.
- `shots/export-1440.png`: Export with an active design, after a Local helper + OpenCascade turn.
- REPORT.md is written.
