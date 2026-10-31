# Cell chat-first: the chat is the centered, primary surface; the drawing appears after a design is built

Read `../../WAVE.md` first. D1 (same design language), D3 (one home per control), D8 and D9 still bind. **D2 is superseded by this cell.** Also read `../integrate/REPORT.md`; that cell finished just before you, and its tests and polish are the baseline.

## User direction (2026-10-31, verbatim)
- "let the chat section stay at the center"
- "rendering is not supposed to be the main thing, the important thing is the chat interaction; the render is supposed to be after the design is done in a CAD tool"

## Store contract (Opus has ALREADY updated `store.ts` and `test/store.test.ts`; do not change them)
At hand-off `tsc` reports errors only in `AppShell.tsx`, `TopBar.tsx`, `test/AppShellChrome.dom.test.tsx` and `test/TopBar.dom.test.tsx`, which still use the removed `chatOpen` / `selectChatOpen` / `chrome/chat-toggle`. Fixing them is part of your task.
- `chatOpen` and `chrome/chat-toggle` are REMOVED: the chat is always shown.
- New state `drawingOpen: boolean`, default `false`.
- New actions:
  - `chrome/drawing-open`, `chrome/drawing-close` and `chrome/drawing-toggle`;
  - `design/received` also sets `drawingOpen: true`, because a CAD-built design opens its drawing.
- New selector `selectDrawingOpen`.

## Layout
1. **Drawing closed: the default, and always so before the first design.**
   - The content area is the chat alone, as a centred column (`max-width` ≈ 760px, `margin-inline: auto`), on the same panel/paper surface as today.
   - **Empty thread:** the composer block (prompt, chips, Draft) sits around the vertical middle of the column, with one short mono-caps label above it and the starter chips under it, as on a new chat in a chat app.
   - **Once the thread has messages:** the thread scrolls in the column and the composer docks at the bottom of the column.
   - **No DraftingSheet / SceneCanvas is mounted** while the drawing is closed.
2. **Drawing open: split view.**
   - Columns: `[chat minmax(400px, 1fr)] [drawing minmax(0, 1.3fr)] [details 360px when open]`. The chat content stays centred inside its column (max 760).
   - The drawing panel is the existing `DraftingSheet` under a slim panel header (mono caps, same style as the Details header). The header holds:
     - the label `DRAWING` and the active design's name, or the primitive name with no design;
     - the **2D Pencil | 3D Clay** segmented control, moved out of the top bar, as its single home (D3);
     - a close button (`aria-label="Close drawing"`).
   - Exactly one SceneCanvas is mounted while the drawing is open.
3. **Top bar.**
   - **Left:** the wordmark only. Remove the chat toggle.
   - **Centre:** empty (the render mode moved).
   - **Right:** `Drawing` (toggle, `aria-pressed`; dispatches `chrome/drawing-toggle`), `Details`, `Export`, ⚙.
   - With no design, `Drawing` still opens the sheet with the primitive playground, so nothing is lost (D8). It is just not shown by default.
4. **Chat → drawing links.**
   - The design result card's summary line gets a `Show drawing` button. It activates that design (`design/activate`) and opens the drawing.
   - The existing `Section A-A ↗` link also opens the drawing, plus Details › Views.
5. **Status bar.**
   - The ENV/VOL/AREA/SCALE readout and the source badge show only while the drawing is open. They describe the drawing, and at boot they described an unasked-for box.
   - The connection label, the FreeCAD dot and the backend version always show.
6. **Keyboard.** `Ctrl/Cmd+B` now toggles the drawing. `Ctrl/Cmd+,` still opens Settings, and the text-field guard is kept.
7. **Responsive.**
   - Below 1280px wide with the drawing open, the chat column min is 360px.
   - Below 1100px, opening Details while the drawing is open overlays Details on the drawing column instead of adding a fourth column. Keep it bounded and inside the existing media-query structure.

## Tests
- Update `AppShellChrome`, `TopBar`, `layoutCss`, `AgentWorkspace`, `ManualConnect`, `AppShellMeshStability` and any other tests that assumed the sheet is always mounted or the render mode is in the top bar. Keep the behaviour intent (D9).
- Add tests for:
  - the default boot shows the centred chat, no SceneCanvas and no readout;
  - `design/received` opens the drawing;
  - Close drawing unmounts the canvas;
  - the `Drawing` toggle opens the primitive sheet when there is no design;
  - `Show drawing` on a card activates the design and opens the drawing;
  - render mode works from the drawing header;
  - `Ctrl+B` toggles the drawing.

## Gates
- `npx tsc -p tsconfig.json --noEmit` shows 0 errors.
- `npx vitest run` is all green; give the count.
- `npm run build` is OK.
- **Screenshots** in `cells/chat-first/shots/` (port 8797, `tools/serve.sh`), at 1440×900 unless a size is given:
  - `empty`: boot, an empty thread, composer in the middle;
  - `thread-no-drawing`: after a turn, with the drawing closed again via ×;
  - `design-split`: right after a Local helper + OpenCascade L-bracket turn (CAD chip → OpenCascade → Esc → type `L-bracket 100 x 80 x 60 mm, 8 mm wall` → Draft → wait 15 s);
  - `design-split-details`;
  - `design-split-1280x800`;
  - `slate-design-split`;
  - `3d-clay`.
- `REPORT.md` with the files, the gate output, the screenshots, and anything not done.

The rules in WAVE.md apply (temp HOME, no commits, no deps, no backend changes, never read credential files, never run the real claude/codex/opencode CLIs). `npm run build` writing `studio/dist` is allowed.
