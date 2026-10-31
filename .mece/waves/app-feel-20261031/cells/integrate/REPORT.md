# Cell integrate — REPORT (app-feel wave, 2026-10-31)

Worker: integration cell, the only worker in this run. Checkout:
`/home/v9i/Projects/Kala-Agent/studio` (shared with the shell, composer and
settings cells). No commits, no stash, no reset, no backend changes, no new
npm dependencies. `src/styles/tokens.css`, `src/state/store.ts` and
`src/types/studio.ts` were not changed by this cell.

## Result

The 29 whole-app failures are rewritten against the app frame (not deleted):
58 test files / **843 tests pass**, `tsc` is clean, `npm run build` succeeds.

The five polish issues from `../../after/` are fixed without changing the
visual language:

- **a. composer chips** — the CAD chip shows the short target only
  (`CAD: OpenCascade`, `CAD: FreeCAD`, `CAD: choose`), sizes to its content,
  and the model chip takes the remaining width. `(opt-in)` and the provider
  version stay in the popover and in both chips' `title`; aria-labels are
  unchanged.
- **b. Details › Properties** — one geometry summary with a new
  `SURFACE AREA` row (same source and `—` honesty as the status bar), one
  "Closed-form nominal geometry…" sentence under it, and the Inspector's
  duplicate estimate card and bounds block hidden via
  `Inspector showReadouts={false}`. The vernier sliders stack in one column
  inside Details, so the full labels fit.
- **c. Details › Views** — the section caption wraps
  (`OFFSET 4.00 mm` / `1 OUTER · 0 INNER LOOPS`) instead of colliding with
  `KERNEL AREA`; the sub-tab row and `CUT: 45° ANSI` are on separate rows at
  every width.
- **d. sheet captions** — the HLR caption block is constrained to the space
  right of the title block and wraps there; the footnote moved to the bottom
  right, above the view controls, clear of the title block at 1440×900 and
  1280×800, with Details open and closed.
- **e. Activity tab** — `JournalPanel` gained `showFooter?: boolean`
  (default `true`); Details › Activity passes `false`, so the origin and
  backend version are not repeated when the status bar already shows them.

One small extra fix found while checking the required `3d-clay` shot: the
design cut note (`CUT A–A · XZ · Y = 4.00 mm`) started under the sheet tool
rail. `.drafting-cut-note.is-design` now starts at `left: 68px`, after the
rail. Tokens only; no visual language change.

## Files changed by this cell

| File | Change |
|---|---|
| `test/AgentWorkspace.dom.test.tsx` | **Rewritten** (23 tests, was 23 incl. 20 failing). Same behaviours, new homes: app frame, status bar, Settings dialog, Export dialog, Details › Views / Properties / Activity, composer chips, Settings › Models & keys keys, journal appenders. |
| `test/layoutCss.test.ts` | **Rewritten** first suite: 4-zone invariants → app-frame invariants (44 px top bar, content row, 28 px status bar, 360 px chat/Details tracks only when mounted, `minmax(0,1fr)` sheet, compact media queries, export panel natural height). Theme/CSP suites unchanged. |
| `test/ManualConnect.dom.test.tsx` | Two connection assertions moved from the (now Details-only) journal footer to the status bar connection label/title. |
| `test/DetailsPanel.dom.test.tsx` | Added SURFACE AREA, hidden readouts, one-note assertions and the no-footer Activity assertion. |
| `test/ChatPanel.dom.test.tsx` | Added short CAD-chip / full title assertions and model-chip title coverage. |
| `src/components/chat/ChatPanel.tsx` | Short CAD chip value + full chip title via `cadProviderShortName`; model chip title. |
| `src/components/chat/ComposerPopover.tsx` | Optional `title` prop on the chip. |
| `src/components/inspector/Inspector.tsx` | `showReadouts?: boolean` (default `true`); exported `ESTIMATE_NOTE`. |
| `src/components/shell/ActiveGeometrySummary.tsx` | `SURFACE AREA` row for primitives and provider-measured designs (`—` when unreported). |
| `src/components/shell/DetailsPanel.tsx` | `showReadouts={false}`, one note under the summary for primitives, `showFooter={false}` on the journal. |
| `src/components/journal/JournalPanel.tsx` | `showFooter?: boolean` (default `true`). |
| `src/components/drafting/AuxView.tsx` | Section caption split onto two SVG lines. |
| `src/viewer/HlrDrawing.tsx` | Footnote moved to the bottom right (`SVG_WIDTH - 24`, `SVG_HEIGHT - 60`, end-anchored). |
| `src/styles/composer.css` | Model chip `flex: 1 1 0`; CAD chip content-sized; chip title text kept in the DOM. |
| `src/styles/shell.css` | Geometry label column 86 px, `.details-geometry-note`, one-column `.details-panel .inspector-grid`. |
| `src/styles/drafting.css` | Aux header always wraps the cut note under the tabs; HLR caption left bound + wrapping; design cut note clears the rail. |
| `src/styles/settings.css` | `.export-dialog-body > .export-panel { min-height: max-content; }`. |

No test was deleted; the behaviour assertions were re-pointed at the control's
one home (D3/D9). `designSystem.test.ts` remains green with no change needed
by this cell.

## Gates actually run

All commands in `studio/` with `RUSTUP_HOME`, `CARGO_HOME`,
`npm_config_cache` and a throwaway `HOME` / `KALA_AGENT_DATA_DIR` /
`KALA_STUDIO_DATA_DIR` / `XDG_DATA_HOME`.

- `npx tsc -p tsconfig.json --noEmit` → **exit 0 (0 errors)**.
- `npx vitest run` → **58 files, 843 tests, all passed** (3.04 s).
- `npm run build` → **exit 0**, writes `studio/dist` (allowed for this cell).

The three previously failing files on their own:
`AgentWorkspace + layoutCss + ManualConnect` → **3 files, 52 tests, all passed**.

## Screenshots (1440×900 unless noted, `cells/integrate/shots/`)

| File | Shows |
|---|---|
| `idle-1440.png` | Idle frame; 22 visible controls (shot.mjs count), short `CAD: choose`, Details closed. |
| `design-1440.png` | Local helper + OpenCascade, `L-bracket 100 x 80 x 60 mm, 8 mm wall`; 5/5 checks; full `CAD: OpenCascade`; HLR caption/footnote clear of the title block. |
| `design-details-properties-1440.png` | Details › Properties with a design: one summary (DIMS/ENVELOPE/VOLUME/SURFACE AREA/SOURCE), no duplicate inspector readouts. |
| `design-details-views-1440.png` | Details › Views: tabs + `CUT: 45° ANSI` on separate rows; wrapped section caption, no `KERNEL AREA` overlap. |
| `design-1280x800-details-open.png` | 1280×800, Details open: wrapped HLR caption right of the title block; footnote clear; 320 px Details column. |
| `3d-clay-1440.png` | Same design after the top-bar `3D Clay`; cut note now clears the tool rail. |
| `slate-design-1440.png` | Slate theme after the design turn. |
| `settings-workspace-1440.png` | Settings › Workspace theme switch. |
| `export-1440.png` | Export dialog with the active design first and `EXPORT THE PRIMITIVE INSTEAD` closed. |
| `details-idle-1440.png` (extra) | Idle Details › Properties: SURFACE AREA, one note, one-column verniers, no estimate card/bounds block. |
| `design-details-activity-1440.png` (extra) | Details › Activity: journal inside Details, no footer (status bar owns origin/version). |

Visual build: `npx vite build --outDir /tmp/kala-integrate-dist --emptyOutDir`,
served with `tools/serve.sh 8796 /tmp/kala-integrate-dist`, captured with
`tools/shot.mjs` (real native backend; Local helper + OpenCascade turn in each
design shot). The 1280×800 **Details-closed** case was checked ad hoc in the
same build and also showed no caption/footnote overlap. The 8796 server was
stopped and the `/tmp` build/look dirs removed.

## Left over / notes

- At 1440 the Activity ledger still uses its six-column layout even though the
  Details column is 360 px; `journal.css` compacts it only below a 1400 px
  viewport. It is pre-existing (the wave's five polish items did not include
  it) and is visible in `design-details-activity-1440.png`. Changing it would
  be a separate decision about the ledger's compact trigger.
- `tokens.css` still defines the retired `--zone-*` and `--bottom-bar-height`
  tokens. D1 forbids editing it; no stylesheet still consumes them.
- Details tab-strip arrow-key navigation is still not implemented (shell cell
  noted it); not part of this task.
- The model chip still ellipsizes a long provider name in a 360 px column; per
  task 2a the full text is in its `title` and aria-label, and the CAD chip no
  longer truncates.
