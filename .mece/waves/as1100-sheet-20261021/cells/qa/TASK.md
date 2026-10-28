# Cell qa (Phase C): integrate, retire the dead UI, visual and a11y pass, full gates

Read `../../WAVE.md` in full, `../../SPEC.md`, and EVERY `../*/REPORT.md` (especially "Known gaps" and "Requests"; Opus's adjudication of the Requests is appended to this file when there is one).

## Do
1. **Relocation audit.** Walk the WAVE relocation table row by row. For each "Today" behaviour, name the new component and the test that proves it works there. Any row without both is a defect: fix it within studio/**.
2. **Retire the dead UI.** Delete whatever is no longer mounted (expected: `components/chat/ChatPanel.tsx`, `components/bottombar/**`, `components/options/**`, `components/drafting/AuxView.tsx` and their CSS), plus dead store helpers and dead CSS.
   - Before deleting a test file, move every behaviour it covers into the new component's tests. Then list each deleted test file and where its coverage went.
3. **Removed regions.** Verify in the running app that none of #2, #3, #4, #7, #8, #9, #10, #11, #12, #15, #16, #17 renders.
4. **Visual pass** against SPEC, in Electron or the browser fallback with a backend on a temp data dir (USER DATA rule), at 1440×900 and 1280×800, in both themes.
   - Check: the frame, ticks and arrows; the tab rail on the rule; the dock proportions 28/48/24; the pencil (ferrule, barrel, nib); the graphite string from the nib to the trail port; the popovers' layering; no overlap between the tool strip, view switcher, scale pill and pencil; and text that never clips or overlaps at 1280.
   - Build a real design with the `local` provider and an OpenCascade CAD target (no remote calls, no real CLIs). Use it for the tabs, the paper roll, a sub-tab build, the ortho sheet hover and popover nudge, the mini-map lenses, the trail dots and downloads.
   - Take screenshots into `cells/qa/shots/` (PNG; delete them from /tmp).
5. **Accessibility.**
   - Full keyboard pass: Tab order frame → tabs → tool strip → switcher → canvas → pencil → dock → title block → engineer button.
   - Focus rings, aria labels, focus traps and returns.
   - `prefers-reduced-motion`.
   - Contrast AA in both themes.
6. **No fake data sweep.** Grep for hard-coded numbers, model names, versions and stamps in the new components. Every value must trace to store or backend state, or be an explicit empty state.

## Own
`studio/src/**` and `studio/test/**`, for integration fixes, deletions and polish only. Keep the cells' designs; do not redesign. Do not touch the backend, `studio/electron/**`, `studio/scripts/**`, `electron-builder.json` or `index.html`.

## Gates
`npm run typecheck`, `npm test`, `npm run build` in `studio/`. Report the test counts before and after, and the coverage moves. No commits. Clean up /tmp, including your backend's temp dirs (stop the backend first). Write `REPORT.md`: the relocation audit table, deletions with their coverage moves, screenshots list, a11y findings and fixes, the fake-data sweep result, and gaps.

## Opus findings from the live look (2026-10-21, `opus-shots/*.png`): MUST fix
The flow works end to end: local helper + OpenCascade builds a verified L-bracket (see `design.png` and `design-ortho.png`). These defects remain:
1. **3D fit framing is too tight.** The model runs under the pencil and to the field edges (`vellum-1440.png`, `design.png`). Frame to about 70% of the FREE area: exclude the tool strip, the view switcher and the pencil band (the bottom 84 px).
2. **Graphite Shade shows no visible hatching.** It reads as flat tan clay. Shadow-side faces need clearly visible graphite cross-hatching, and lit faces paper highlights. In slate, faces render near-black (`slate-1280.png`): use blueprint-blue tones with light hatching.
3. **The provenance label collides with the view switcher.** The `VIEW: OPENCASCADE IMPORT (VIEW-O…` label is under it (`design.png`, top right). Move the label into Process Trail Dot 2 (it belongs to provenance), or below the switcher.
4. **Pencil.**
   - The conical nib is not visible: the right end is empty with a faint ▶. Draw the tapered wood cone and the graphite point.
   - The ferrule's CLEAR is illegible and crowds ↺.
   - The barrel text is top-aligned when a design is active (`design.png`); it must be vertically centred.
5. **The scale pill collides with the pencil ferrule** (1280: `slate-1280.png`; nearly at 1440). Per SPEC it sits in the bottom-left corner of the sheet MARGIN: move it into the 20 px margin ring, outside the inner rule.
6. **Ortho sheet.**
   - The `SOURCE: OCP HLR…` footer is under the pencil.
   - The FRONT/RIGHT labels sit just above the pencil band.
   - Reserve the pencil band (bottom 84 px) in the ortho layout and put the source line elsewhere (e.g. under the title).
7. **Mini-map caption** ("The hovered or selected feature of the active design, or i…") truncates. Make it short: e.g. `TOP VIEW · hover a vector to locate it`.
8. **Process Trail.**
   - The milestone labels truncate (`• Intent & sc… — • B-Rep & metro…`). Use compact labels that fit 340 px (e.g. `INTENT — CHECKS — FILES — ◉`), with the full names in a title or aria.
   - Remove the blank band under the trail header.
9. **The audit footer says `0 ENTRIES · VERIFIED`** for an empty chain. A verdict over zero entries is vacuous: show `0 ENTRIES` with no verdict.
10. **The title block's CAD BRIDGE note truncates** (`This design was alr…`). Use a short note (`built here`), with the full reason in the title.
11. **Requests from the cells.**
    - A memoised `selectSessionUsage` (keyed on the `chat.thread` reference) usable directly with `useUiState` (title-block Request 1).
    - Pass `designController` to `EngineerCard`, so the sheet export reuses the drawing cache (title-block Request 2).
    - Drop DraftingSheet's transitional `material` / `renderMode` / `titleBlock` / `units` props, and retire `RenderMode` / `MaterialKind` / `AuxViewKind` / `OptionsCard` with the dead components (canvas-3d).
    - Make scale lock govern the pinch dolly too (canvas-3d).
    - Drop the `data-stub` markers from the real components and rewrite `shellStubs` / `AgentWorkspace` to assert the real components.

## How to run it (Opus used this)
- **Backend.** In a temp dir: `KALA_STUDIO_TOKEN=<any 32 chars> backend/target/release/kala-studio --port <free port> --data-dir $T/studio --static-dist studio/dist`, with the USER DATA env vars set.
  - Rebuild `studio/dist` first (`npm run build`).
  - The release binary may predate edit-turns; that does not matter for the local helper.
- **Screenshots.** `shot.mjs` and `flow.mjs` here: Node + `/home/v9i/.hermes/hermes-agent/node_modules/playwright-core` + `/usr/bin/chromium`.
  - Edit the port and token.
  - The flow selects OpenCascade in the Engineer card § CAD, drafts the L-bracket with the local helper, and opens the 2D view.
- Put new screenshots in `shots/`. Stop the backend and remove your temp dirs at the end.
