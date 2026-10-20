# P3 qa: visual fidelity pass and full gates

Read `../../WAVE.md` and all three REPORTs (`../shell`, `../viewport`, `../chat-journal`). You own all of `studio/src/**` and `studio/test/**` for cosmetic and integration fixes only: no redesign, no new features, no fake data. Same forbidden zones as WAVE.md.

1. Run the gates first: `npm run typecheck && npm test && npm run build`. Fix integration breakage between the P2 cells.
2. Launch the real backend plus the UI the way P1 did (see shell/REPORT.md), using headless Chromium/Playwright screenshots at 1440x900 and 1280x800. Capture: vellum theme default; slate theme; Options drawer open; 2D pencil vs 3D clay; Section toggle on with the AuxView Section/Ortho/Tolerance tabs; one real chat turn if a provider or the local deterministic helper responds (otherwise an honest note).
3. Compare each screenshot against `stitch/kala_studio_the_vellum_drafting_board_master_workspace/screen.png`, `stitch/kala_studio_options_engineering_drawer_wireframe_sheet/screen.png`, and `..._night_slate/screen.png`. Fix spacing, typography, borders, colours, overflow, clipped text, scrollbars, focus rings, and contrast (WCAG AA for body text in both themes).
4. Check the invariants: no remote URLs in `dist/` (`grep -rE "https?://" dist | grep -v w3.org`), CSP unchanged, no console errors, exactly one WebGL canvas, no fake values (grep for 28.16, 14208, 8f3a, "HASH-CHAIN OK" literal outside guarded logic, "1.1.3", "Llama").
5. Save the final screenshots to `/home/v9i/Projects/Kala-Agent/.mece/waves/vellum-ui-20261019/cells/qa/shots/*.png` (name them clearly).
Write `REPORT.md` in this cell folder: fixes made, gate counts, screenshot list, remaining visual gaps compared with Stitch (be honest), and the invariant check results.

## Extra item from P2a (must fix)
The backend response CSP (`src/kala_agent/studio/middleware.py`, `font-src 'self'`) blocks @fontsource fonts that Vite inlines as `data:` URIs in browser mode. Do NOT change either CSP. Instead, in `studio/vite.config.ts`, make the build never inline font files (for example `build.assetsInlineLimit: (file) => /\.(woff2?|ttf|otf)$/.test(file) ? false : undefined`) so every font ships as a same-origin file. Verify that `dist/` has no `data:font` / `data:application/font` URIs and that the browser console shows no CSP font violations.
