# P3 qa — visual fidelity pass and full gates

Status: **complete**. The P1/P2 work integrates, the full gates are green, both
themes were exercised against the real backend in headless Chromium, and the
screenshots are saved in `shots/`. No commit, push, deploy, credential read, or
delegation was performed. Files touched: `studio/src/styles/**`,
`studio/src/components/options/OptionsDrawer.tsx`, `studio/test/OptionsDrawer.dom.test.tsx`,
`studio/vite.config.ts`, and this report — nothing outside `studio/**` plus the
cell folder.

## 1. Gate results

Run from `/home/v9i/Projects/Kala-Agent/studio`:

| Command | Result |
| --- | --- |
| `npm run typecheck` | clean (exit 0) |
| `npm test` | **19 test files passed, 224 tests passed** (P2 baseline 18/220; +4 new QA tests) |
| `npm run build` | success, 50 modules transformed; only the pre-existing >500 kB `three` chunk warning |

The build ships 15 same-origin `.woff2` files (previously one small subset was
inlined as a `data:` URI — see fix 1).

## 2. Fixes made (cosmetic / integration only)

### 2.1 The @fontsource CSP mismatch (P2a hand-off, mandatory item)

`studio/vite.config.ts` now never inlines font files:

```ts
assetsInlineLimit: (filePath: string) =>
  /\.(?:woff2?|ttf|otf|eot)$/i.test(filePath) ? false : undefined,
```

This satisfies the backend response header `font-src 'self'` without changing
either CSP. Other small assets keep Vite's default inlining (the inline SVG
grid/noise data URIs are still there, allowed by `img-src 'self' data:`).

Verified after the change:

- `grep "data:font\|data:application/font" dist/assets/*.css` → **0 matches**;
- 15 `.woff2` files emitted under `dist/assets/` and referenced relatively
  (`url(./space-grotesk-latin-wght-normal-*.woff2)`);
- browser console shows **no CSP violation** and all three families report
  loaded (`Space Grotesk Variable normal`, `Newsreader Variable italic`,
  `JetBrains Mono Variable normal`);
- all 12 page requests in a fresh session went to `http://127.0.0.1:8765` only.

### 2.2 WCAG AA for small text in both themes

The browser audit (every visible text node, effective background walk,
4.5:1 threshold for small text) flagged 31 unique rows in vellum 1440
(51 with the drawer open) and 5 in slate. The fix keeps the drafting look but
raises the contrast:

- vellum `--construction` `#7a756c` → `#645f55` (same graphite family):
  sheet 4.13 → **5.72:1**, panel 3.88 → **5.37:1**, boxwood ruler bar
  3.51 → **4.86:1**.
- new `--redline-ink` token used for redline **text** only (dimensions, cut
  note, error stamps, export tag); the `--redline` line colour is unchanged
  (`#c84b31`, pinned by `layoutCss.test.ts`). Vellum text red `#b8432b`:
  **4.89:1** on the sheet, 4.59:1 on the panel; slate keeps `#dd5a3e` (4.9:1+).
- slate `--prussian` `#33628b` → `#9ecbfa` (the light-blue accent already in
  the night-slate Stitch palette): active drawer nav / connection tags /
  provider statuses went 2.80–3.07 → **10.6–11.0:1**.
- `.drafting-mode-divider` was `--hairline` (1.6:1) and now uses the
  construction colour.

After the change the same audit returns **zero** failing rows in vellum 1440,
vellum 1280, slate 1440, and with the Options drawer open in both themes.

### 2.3 Options drawer card jump (integration bug found by measurement)

Clicking a drawer nav item used `scrollIntoView({ block: 'nearest' })`. The
grid rows are as tall as the tallest card (card 01 is ~1306 px tall because the
row matches the registry card), so selecting card 01 left the grid at
`scrollTop 597` with the entire card content above the fold — the nav looked
selected but the operator saw empty space. Now `block: 'start'` aligns the card
header with the drawer viewport (`scrollTop` returns to 10/1064 as expected).

### 2.4 Options card notes no longer hardcode Stitch state

Card 02 said `ISO 10303 READY` and card 04 said `PORT 8765` unconditionally.
Both now read the live store: the port comes from the ready connection origin
(`NO LOOPBACK` before connect), and the export note follows
`server.stepAvailable` (`STEP —` / `ISO 10303 READY` / `STEP UNAVAILABLE`).
Screenshots show `PORT 8765` and `ISO 10303 READY` because that is what this
backend actually reported.

### 2.5 New regression test

`studio/test/OptionsDrawer.dom.test.tsx` (4 tests): real port note, explicit
`NO LOOPBACK`, STEP capability note, and the `block: 'start'` scroll contract.
No existing assertion was changed or deleted.

## 3. Screenshots (`shots/`)

All captured in headless Chromium against the real `.venv` backend on
`127.0.0.1:8765` serving `studio/dist`, connected through the normal
browser-fallback no-persistence form.

| File | Viewport | Shows |
| --- | --- | --- |
| `01-vellum-1440-default.png` | 1440×900 | Vellum default, 2D pencil, Section A–A on, server-verified mesh |
| `02-vellum-1440-clay3d.png` | 1440×900 | 3D clay solid mode (same session, one WebGL canvas) |
| `03-vellum-1440-ortho.png` | 1440×900 | AuxView **Ortho 3-View** with third-angle rule |
| `04-vellum-1440-tolerance.png` | 1440×900 | AuxView **Tolerance** honest empty state |
| `05-vellum-1440-section-off.png` | 1440×900 | Section A–A toggled off (flags + cut note removed) |
| `06-vellum-1440-options.png` | 1440×900 | Options drawer, cards 01/02 |
| `06b-vellum-1440-options-providers.png` | 1440×900 | Options drawer, card 03 |
| `06c-vellum-1440-options-connection.png` | 1440×900 | Options drawer, card 04 |
| `07-slate-1440-options.png` | 1440×900 | Slate theme with the drawer open |
| `08-slate-1440-default.png` | 1440×900 | Slate night theme, full workspace |
| `09-slate-1280-default.png` | 1280×800 | Slate at the small desktop size |
| `10-vellum-1280-default.png` | 1280×800 | Vellum at the small desktop size |
| `11-vellum-1280-options.png` | 1280×800 | Drawer at 1280 (scrollable modal) |
| `12-vellum-1280-chat-turn.png` | 1280×800 | One **real** local-helper chat turn + journal rows |
| `13-vellum-1440-chat-turn.png` | 1440×900 | The same real turn at the reference size |

## 4. Browser verification (evidence, not just screenshots)

- **Layout**: 1440 → `340 / 780 / 320`, sheet 852 + 48 px bar, AuxView exactly
  250; 1280 → `280 / 740 / 260`, sheet 752, `documentElement` scroll
  `1440×900` / `1280×800` with **0 overflow**; no unexpected inner scrollbars
  (only the drawer grid and the journal/chat lists, all bounded).
- **One canvas**: exactly one `<canvas>` with a live WebGL context through mode
  switches, theme switches, drawer opens, and AuxView tab changes.
- **Themes**: `[data-theme]` switches, palette applies to sheet/grid/axes/mesh/
  dims/journal, and only `localStorage['kala-studio.theme']` is written
  (`Object.keys(localStorage)` = that one key; no origin, token, or content).
- **Drawer a11y**: `role="dialog"`, `aria-modal="true"`, focus moves to the
  close button, 20 consecutive Tabs never leave the dialog, Esc closes and
  focus returns to the `.bottom-bar-options` trigger; focus ring is 2 px
  `--focus-ring` (Prussian in vellum, light slate-blue at night).
- **Real turn**: prompt sent to the local deterministic helper; user + agent
  cards rendered, journal gained `[CHAT]`/`[TOOL]` rows, no error element.
- **Integration toggles**: Grid switch flips, Dims toggles the real
  `W 40.00 mm / H 20.00 mm / D 30.00 mm` overlay, `Export STEP` opens the
  drawer on card 02 with focus in it, Reconnect/Retry re-loads health (OK).
- **Console**: only four Chromium `GL Driver Message … GPU stall due to
  ReadPixels` performance warnings; **no console errors, no page errors, no
  failed requests**.

## 5. Invariant checks

| Check | Result |
| --- | --- |
| `grep -rE "https?://" dist` | Matches exist, but **no remote resource is referenced**. Shipped JS contains only loopback literals (`http://127.0.0.1`, `http://localhost`, required by the connection form/copy), React's error-documentation string (`https://react.dev/errors/`), and one three.js GLSL citation comment (`https://jcgt.org/…`). Source maps additionally carry three.js documentation comments. A strict asset check (`src=`/`href=`/`url(`/`@import` pointing at `https?://`) and a CDN check (`cdn.tailwindcss`, `fonts.googleapis`, `fonts.gstatic`, `lh3.googleusercontent`) both return **no matches**, and the browser session requested the loopback origin only. |
| `dist` font URIs | **0** `data:font` / `data:application/font`; 15 `.woff2` same-origin files. |
| CSP unchanged | `studio/index.html` meta and `src/kala_agent/studio/middleware.py` header were **not edited**; served header still `font-src 'self'` with `default-src 'self'`, `frame-ancestors 'none'`, etc. No CSP warning in the console. |
| No console errors | only the four WebGL driver performance warnings. |
| Exactly one WebGL canvas | yes (verified in 3 states). |
| Fake values | `grep -rnE "28\.16|14208|8f3a|HASH-CHAIN OK|1\.1\.3|Llama" src` → only `JournalPanel.tsx` line 222, the guarded `status === 'verified'` branch (plus its comment). `AppShell` passes `hashChainStatus="unknown"`, so the UI renders `SESSION LOG`; the literal is unreachable in this build. No `FIPS`, `keytar`, `B-Rep Verified`, or fabricated PID/latency anywhere. |

## 6. Remaining visual gaps vs the Stitch screens (honest)

1. **Part shape.** The Stitch vellum sheet shows an L-bracket with fillets and
   bores. This build's parametric contract is box/cylinder/sphere, so the
   screenshots show the active primitive (a box). No geometry is faked to look
   like the reference.
2. **Chat content.** Stitch shows two agent cards with a resolved parameter
   table, digest chip, and action buttons. The real backend `ChatResponse`
   carries only `text/provider/model/offline`, so the UI shows the returned
   prose and a real active-geometry table, not a parsed agent result.
3. **Journal density.** Stitch's ledger has many rows with digests and
   `[partdesign.…]` categories. The store's real kinds are
   `preview/chat/tool/export/connection/system`, and digests appear only when a
   real response carries them; the default screenshots carry four real rows and
   the after-chat screenshot carries seven.
4. **Tolerance block.** Stitch prints `ISO 2768-m` and `03 [STABLE]`. The
   backend exposes no tolerance/fit data, so the sheet and the Tolerance tab
   say `—` / `not provided`.
5. **Verification stamps.** `HASH-CHAIN OK` and `B-Rep Verified` are not
   reachable without a real backend report; the header says `SESSION LOG`.
6. **Options sheet shape.** Stitch's drawer is a full-page A2 sheet with all
   four cards visible. The WAVE brief asks for a modal, so this is a centred
   modal with a 2-column grid and an internal scroll; cards 03/04 need a
   scroll/jump (now fixed to land on the card header).
7. **Density/typography.** The app uses 7–9 px mono labels to match the
   drafting language; Stitch's mock is rendered at a slightly larger UI scale,
   so side-by-side screenshots look finer-grained. No structural change was
   made.
8. **Grid texture.** The sheet uses a 50 px drafted grid with `+` ticks and an
   SVG grain; Stitch's grid is denser and has faint construction lines. Purely
   decorative.
9. **Night theme layout.** The night_slate export is a different 3-panel
   workbench; per WAVE only its palette was adopted, so the layout stays the
   4-zone vellum board at night.
10. **Bottom bar extras.** Stitch's `Slide-Rule Companion Overlay` hint and
    `[ ] Export` chip have no implemented feature behind them and were not
    fabricated; the bar shows only real toggles, mode, readout, FreeCAD
    registry status, and the drawer trigger.
11. **Clay colour.** The 3D clay solid is a warm saturated clay (Three.js
    lighting); it is a render mode with no direct Stitch counterpart.

## 7. Anything left fake or stubbed

Nothing invented was added by this cell. The only static labels are explicit
author/org text in the title block (`DRAFT`, `Kala Studio`) and descriptive
notes (`REAL STORE STATE`, `[LOCAL VAULT]`, `NO CERTIFIED FEA RESULT`), which
are labels rather than data claims. Every number, status, endpoint, receipt,
and journal row shown in the screenshots comes from the live store, the active
parameters, or the backend response. Empty states are explicit: `—`,
`not provided`, `NO LOOPBACK`, `STEP —`, `unavailable`.

## 8. Files changed by this cell

| File | Change |
| --- | --- |
| `studio/vite.config.ts` | `build.assetsInlineLimit` never inlines font files |
| `studio/src/styles/tokens.css` | darker vellum construction grey; `--redline-ink` in both themes; lighter slate Prussian blue |
| `studio/src/styles/{chat,drafting,journal,layout,studio}.css` | 22 redline **text** colours point at `--redline-ink`; drafting mode divider contrast |
| `studio/src/components/options/OptionsDrawer.tsx` | `scrollIntoView({block:'start'})` for card jumps; card 02/04 notes from live store state |
| `studio/test/OptionsDrawer.dom.test.tsx` | new 4-test regression suite for the two integration fixes |
| `.mece/waves/vellum-ui-20261019/cells/qa/shots/*.png` | 15 final screenshots |
| `.mece/waves/vellum-ui-20261019/cells/qa/REPORT.md` | this report |
