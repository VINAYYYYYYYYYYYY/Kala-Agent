# C0b — design-system pass

Cell `c0b-system`, wave `vellum-ui-20261019`. Tier 1 items 1–4 of
`../../CONSOLIDATED-GAPS.md`: type scale, spacing/radius, the Options drawer
as a drafted 2×2 sheet, and the Vernier slider; plus the Opus addendum for a
single-row bottom bar.

Status: **complete**. All gates are green and the four listed screenshots are
in `shots/`. No commit, push, deployment, credential read, delegation, or
Python edit was performed.

---

## 1. Files changed

### New — `studio/src/**`

| File | Contents |
| --- | --- |
| `components/inspector/VernierSlider.tsx` | The caliper control: bounded tick rail, triangular thumb, visible MIN/STEP/MAX, −/+ steppers, mono value input, `role="slider"` with `aria-valuemin/max/now`, arrows/PageUp/PageDown/Home/End, pointer scrubbing. Exports `computeVernierTicks` and `snapToStep` as pure helpers. |

### Modified — `studio/src/**`

| File | Change |
| --- | --- |
| `styles/tokens.css` | The full DESIGN.md type scale (`headline` 22/28, body 16/14/12, labels 13/11/9 with letter-spacing), `--space-xs…xl` (4/8/12/20/32), `--gutter`/`--gutter-mobile`, and `--radius: 0`. These are the only px `font-size` values in the app. |
| `styles/base.css` | Body text now uses `--text-body-md` (14 px); control padding/radius use the spacing and radius tokens. |
| `styles/layout.css` | Ruler bar is a non-wrapping flex row. Icon-button styling for the view toggles, fixed readout order, ellipsis + title behaviour at 1280, material control hidden below 1600 px, compact 1400 px rules. |
| `styles/studio.css` | 2×2 drafter grid, four corner ticks, card icons and drafted card headers, real-data title block, compact card bodies, the complete Vernier CSS, and the Inspector two-column lower row. Canvas is constrained to its container (`width/height: 100%`) so a DPR-2 browser does not render it at twice its CSS size. |
| `styles/chat.css` | Type tokens; header stamp fits on one line at 280 px. |
| `styles/journal.css` | Type tokens; 9 px ledger metadata, 12 px messages; 1280 header size. |
| `styles/drafting.css` | Type tokens; at 1280 the Aux header puts the tabs on one row and the redline CUT note on the next so neither clips. |
| `components/bottombar/BottomBar.tsx` | Grid/Axes/Dims/Section/Reset are graphite line-icon buttons with `aria-label` + `title`. Readout is one mono line in the order **ENV · VOL · AREA · SCALE · source**. FreeCAD is a compact status chip (`FreeCAD` visible; real status in `aria-label` and `title`). |
| `components/options/OptionsDrawer.tsx` | Four card icons, card 01–04 numbering, 4 corner ticks per card, card bodies, real-data footer title block (sheet id, drawing title, NTS scale, backend version, connection origin), live inspector source note. Focus trap, Esc, and focus return are unchanged. |
| `components/inspector/Inspector.tsx` | Optional `showHeading` (drawer suppresses the duplicate `INSPECTOR` heading); native ranges replaced by four `VernierSlider`s; estimates and bounds share a lower row; the honesty note is full-width; invalid fields expose `aria-invalid` and an alert. |
| `components/export/ExportPanel.tsx` | Optional `showHeading` (drawer suppresses the duplicate `EXPORT & HANDOFF` heading). |
| `components/panels/ProviderPanel.tsx` | In compact mode, provider notes stay available as the row tooltip instead of consuming four extra lines. Full mode still prints them. |

### New / modified — `studio/test/**`

| File | Change |
| --- | --- |
| `test/designSystem.test.ts` | **New guard suite (11 tests).** Fails if any CSS file outside `tokens.css` declares a raw px `font-size`, if any type declaration outside `tokens.css` is not a `--text-*` token, if any declared size (including token values) is below 9 px, if the spacing/radius tokens disappear, or if a rounded control corner appears. Also pins the 2×2 drawer, `align-self: start`, four corner ticks, and the bar’s `flex-wrap: nowrap`. |
| `test/VernierSlider.dom.test.tsx` | **New (9 tests).** Tick derivation (including a 0.1…2000 range), major/minor marks, endpoint/step snapping, ARIA values, thumb, MIN/STEP/MAX labels, steppers and clamps, all keyboard keys, the mono input, and the Inspector integration (no native `type=range`; keyboard writes `parameters.size_x`). |
| `test/OptionsDrawer.dom.test.tsx` | +3 tests: four icons and 16 corner ticks, no nested duplicate `INSPECTOR`/`EXPORT & HANDOFF` headings, and the footer title block using the live backend version/origin with no fake ratio. |
| `test/BottomBar.dom.test.tsx` | +2 tests: fixed readout order and icon-only toggles with labels/titles/tooltips. |
| `test/AgentWorkspace.dom.test.tsx` | Updated the old text-based toggle lookup to use `aria-label` (the toggles are now icons) and asserted the real FreeCAD status through its label/title. No coverage deleted. |

---

## 2. Contract added

```ts
// components/inspector/VernierSlider.tsx
export interface VernierTick {
  value: number;
  ratio: number;        // 0 at min, 1 at max
  major: boolean;
}
export interface VernierSliderProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  unit?: string;
  integer?: boolean;
  invalid?: boolean;
  onChange(next: number): void;
}
export function computeVernierTicks(min: number, max: number, step: number): VernierTick[];
export function snapToStep(value: number, min: number, max: number, step: number, integer?: boolean): number;

// components/inspector/Inspector.tsx
export interface InspectorProps { showHeading?: boolean }

// components/export/ExportPanel.tsx
interface ExportPanelProps { showHeading?: boolean }
```

No store fields were added or changed. The drawer still uses the existing
`options/open`, `options/card`, `options/close`, `theme/set`, and
`connection/retry` actions.

---

## 3. Type scale — before / after

Method: regex over `studio/src/styles/*.css` (`font-size: <number>px`). The
"before" column is the pre-cell tree; the "after" column is this working tree.

| Metric | Before | After |
| --- | ---: | ---: |
| `font-size` declarations | 188 | **194** (new Vernier/icon CSS) |
| Raw px declarations outside `tokens.css` | 188 | **0** |
| Declarations below 9 px | **112 (59.6 %)** | **0** |
| Declarations at 7.5 px or smaller | 47 | **0** |
| Smallest declared size | **6.5 px** | **9 px** |
| Distinct sizes | 14 | 7 tokens (6 in use + headline) |
| Largest size | 20 px | 22 px (headline) |

Token use after the pass:

| Token | px / line | Declarations |
| --- | --- | ---: |
| `--text-label-sm` | 9 / 12 | 124 |
| `--text-label-md` | 11 / 14 | 34 |
| `--text-body-sm` | 12 / 16 | 25 |
| `--text-label-lg` | 13 / 16 | 7 |
| `--text-body-md` | 14 / 20 | 2 |
| `--text-headline` | 22 / 28 | 2 |

Before distribution: `6.5×3 · 7×12 · 7.5×32 · 8×46 · 8.5×19 · 9×23 ·
9.5×4 · 10×19 · 10.5×6 · 11×17 · 11.5×1 · 12×3 · 13×2 · 20×1`.
(Audit C0 quoted 125 below 9 px; this working tree measured 112 with the
stricter `font-size` declaration scan.)

Density was rebalanced by **tightening padding**, not shrinking text: the
bar uses 24 px icon buttons and a 1 px readout gap; card bodies use 2–4 px
gaps; the Inspector's four vernier fields use a 16 px rail only inside the
drawer while the standalone control keeps the full 22 px rail.

---

## 4. The four items

### 1. Type scale — done

All 194 declarations now use `--text-*` tokens. Nothing is below 9 px. The
drawer title uses the 22/28 headline; body copy is 14 px (standalone) or
12 px (dense panels); labels/meta are 13/11/9 px mono. `test/designSystem.test.ts`
enforces both rules and will fail on a reintroduced `font-size: 7px`.

### 2. Spacing and radius — done

`--space-xs: 4`, `sm: 8`, `md: 12`, `lg: 20`, `xl: 32`; `--gutter: 12`,
`--gutter-mobile: 8`, `--sheet-margin: 16`; `--radius: 0`. Structural
padding/gap values moved onto the tokens in the shell, ruler bar, panels,
Inspector, and drawer. The only `50 %` radii left are the circular status
indicator dots; every control/card corner is square.

### 3. Options drawer as a drafted 2×2 sheet — done

* 2 columns × 2 rows; at 1440×900 the measured drawer is 828 px tall and the
  grid is **702 px scrollHeight = 702 px clientHeight** (no outer scroll).
* All four card bodies measured `scrollHeight === clientHeight` at 1440 for
  cards 01/03/04; card 02's 11-row integration registry has its own bounded
  list scroll (the card itself is fully visible).
* Cards are `align-self: start` and `max-height: 100%`, so card 01 is not
  stretched by card 02 (measured heights 335 / 339 / 288 / 241 px).
* Card 01 no longer prints a nested `INSPECTOR` heading and card 02 no longer
  prints `EXPORT & HANDOFF`; the card header is the only heading.
* Four corner `+` ticks per card (16 total), card icons (settings, output,
  key, sync), index + title headers.
* Footer title block: `STUDIO`, `SHEET ID KALA-UI-OPT-04`, drawing title,
  `SCALE NTS`, `BACKEND v0.1.0` (live), `CONNECTION ORIGIN`
  `http://127.0.0.1:8765` (live). No `1:1` claim, no fake revision/approval.
* Focus trap, Esc close, and focus return are unchanged; their existing tests
  still pass.

### 4. Vernier slider — done

The Inspector now renders four `VernierSlider`s (box), three (sphere), etc.
No native range input remains. Every rail exposes
`aria-valuemin`/`aria-valuemax`/`aria-valuenow` plus `aria-valuetext`; the
steppers and mono input write the same parameter state; the keyboard contract
is arrows ±1 step, PageUp/PageDown ±10 steps, Home = min, End = max.

### Added by Opus — single-row bottom bar — done

* Left-cluster Grid/Axes/Dims/Section and Reset are inline-SVG graphite line
  icons with `aria-label` and `title`.
* Readout is one mono line: `ENV · VOL · AREA · SCALE · source`.
* At 1440 all four values are fully rendered (scrollWidth = clientWidth in
  browser measurement); at 1280 they ellipsize by 2–3 px and every item
  carries the full `label value — description` in its `title`.
* The compact material control is hidden in the bar below 1600 px; Options
  card 01 carries it there. At 1440 the bar measures exactly 48 px and
  `flex-wrap: nowrap`.

---

## 5. Gates

Run from `/home/v9i/Projects/Kala-Agent/studio`:

| Command | Result |
| --- | --- |
| `npm run typecheck` | **green**, exit 0, no output |
| `npm test` | **33 test files passed, 440 tests passed** (0 failed) |
| `npm run build` | **green**, 58 modules transformed; only the pre-existing >500 kB `three` chunk warning |

The new suites are `designSystem.test.ts` (11) and
`VernierSlider.dom.test.tsx` (9); the existing OptionsDrawer/BottomBar/
AgentWorkspace suites gained 5 tests. No test was deleted.

---

## 6. Screenshots (`shots/`)

Captured at exact viewport sizes with headless Chromium over CDP against the
real `.venv` backend (`python -m kala_agent.studio`, `127.0.0.1:8765`,
`studio/dist`), connected through the normal browser-fallback form.

| File | Viewport | Shows |
| --- | --- | --- |
| `01-vellum-1440-default.png` | 1440×900 | Vellum default with the one-line bar and all four readout values |
| `02-vellum-1440-drawer.png` | 1440×900 | Options drawer 2×2, all four cards visible, inspector body fully fitted |
| `03-slate-1440-drawer.png` | 1440×900 | Night-slate drawer with the same layout and real title-block data |
| `04-vellum-1280-default.png` | 1280×800 | Small desktop: single-row bar with ellipsized values, wrapped Aux header, no document overflow |

Browser measurements (CDP, not inferred from CSS):

| Check | 1440×900 | 1280×800 |
| --- | --- | --- |
| document overflow | 0 | 0 |
| drawer grid outer scroll | 0 (702 = 702) | allowed/auto below 1400 |
| bar height / wrap | 48 px / nowrap | 48 px / nowrap |
| canvas CSS size | 780×852 | 740×752 |
| readout full values | 4/4 | ellipsis, full value in title |
| card 01 scroll | 0 (294 = 294) | natural height, sheet scrolls |

---

## 7. Invariant / honesty checks

* No fake sheet values: the title block prints the live backend version and
  the live connection origin, or `—`.
* No `1:1 NOMINAL`, PID, latency, SHA, FIPS, or FreeCAD version is printed.
* `index.html` CSP, `electron/**`, `scripts/**`, `electron-builder.json`,
  and `src/kala_agent/**` were not edited by this cell.
* No remote fonts or resources were added.
* The only non-font layout workaround outside the design layer is the
  `canvas { width: 100%; height: 100% }` CSS rule: without it a DPR-2 browser
  displayed the Three.js canvas at its drawing-buffer size (1560×1704) and
  clipped the model. The renderer's pixel ratio is unchanged.

## 8. Known gaps (honest)

1. **Card 02 registry scrolls internally.** The live backend registers 11
   integrations; the 118 px bounded list shows three rows at a time. All rows
   remain reachable and the card box is fully visible. This is the only
   intentionally bounded content at 1440.
2. **1280 truncation is deliberate.** The bar and title-block values use
   ellipsis, with the full value in the `title` tooltip in the bar. This is
   the Opus addendum's stated rule.
3. **At 1280 the drawer scrolls.** All four cards remain reachable; the
   no-scroll guarantee is scoped to 1440×900.
4. **Vernier rails are 16 px tall inside the drawer** (22 px standalone) so
   the 2×2 sheet fits; the ticks, thumb, steppers, labels, and ARIA contract
   are identical.
5. **Circular status dots keep `border-radius: 50%`.** They are indicator
   dots, not rounded control corners; every card/button/input corner is 0.
