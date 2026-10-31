# Cell chat-first — REPORT (app-feel wave, 2026-10-31)

Worker: `chat-first`, the only worker in this run. Checkout:
`/home/v9i/Projects/Kala-Agent/studio`. No commits, no stash, no reset, no
backend (Rust/Python) changes, no new npm dependencies. `src/styles/tokens.css`,
`src/state/store.ts`, `src/types/studio.ts` and `test/store.test.ts` were not
changed by this cell.

## Result

The chat is now the centred, primary surface; the drawing is a secondary
split panel.

- **Boot:** the content area is the chat alone. Its content is capped at a
  760px frame and centred (`margin-inline: auto`). The empty thread shows one
  short mono-caps label (`New design`), the composer block around the middle
  of the column, and the four starter chips under it. No `DraftingSheet`,
  no `SceneCanvas` and no status-bar readout are mounted.
- **Thread:** once the thread has messages it scrolls in the column and the
  composer docks at the bottom of the same 760px frame. `New chat` brings the
  empty stage back.
- **Drawing open:** `[chat minmax(400px,1fr)] [drawing minmax(0,1.3fr)]`
  plus `[details 360]` when Details is open. The drawing is the existing
  `DraftingSheet` under a slim header: `DRAWING` + the active design's name
  (or the primitive name), the only `2D Pencil | 3D Clay` control, and
  `Close drawing`.
- **Opening/closing:** the drawing is closed by default, opens automatically
  on `design/received`, opens from the top-bar `Drawing` toggle or `Ctrl/Cmd+B`
  with no design (primitive playground), and unmounts the sheet and canvas on
  close.
- **Chat → drawing links:** the design card's collapsed summary line has a
  `Show drawing` button (activates that design + opens the drawing). The
  `Section A–A ↗` pill also opens the drawing and Details › Views.
- **Status bar:** ENV/VOL/AREA/SCALE and the source badge show only while
  the drawing is open. The connection label, FreeCAD dot and backend version
  always show.
- **Responsive:** below 1280 the split view keeps a 360px chat minimum; below
  1100 Details overlays the drawing column (bounded to the content row)
  instead of adding a fourth track. A container query on the drawing panel
  drops the HLR caption's secondary view/meta lines and decorative footnote
  when the panel is narrower than 760px (e.g. 1440 with Details open), so the
  title and redline cut note stay legible.

## Files changed by this cell

| File | Change |
|---|---|
| `src/components/shell/AppShell.tsx` | Chat-first frame: chat always mounted, drawing rendered only while `drawingOpen`, `data-drawing`/`data-details`, `Ctrl/Cmd+B` toggles the drawing, drawing name from the active design or primitive. |
| `src/components/shell/TopBar.tsx` | Wordmark only on the left, empty centre, right `Drawing` (aria-pressed), `Details`, `Export`, ⚙. Removed the chat toggle and the render-mode group. |
| `src/components/shell/DrawingPanel.tsx` | **New.** Slim panel header (`DRAWING`, name, only render-mode control, `Close drawing`) over `DraftingSheet`. |
| `src/components/chat/ChatPanel.tsx` | Centred 760px frames; empty stage (label + centred composer + starters) vs. scrolling thread with docked composer; no behaviour change to send/consent/chips. |
| `src/components/chat/DesignAgentCard.tsx` | `Show drawing` on the collapsed summary; `Section A–A ↗` opens the drawing and Details › Views. |
| `src/components/bottombar/BottomBar.tsx` | Readout and source badge only while `drawingOpen`; connection/FreeCAD/version always. |
| `src/components/drafting/DraftingSheet.tsx` | Comment only (render mode now in the drawing panel header). |
| `src/styles/layout.css` | Chat-first grid, chat/drawing columns, 760px content frame support, 1280 chat minimum, 1100 Details overlay, drawing container. |
| `src/styles/shell.css` | Drawing panel header/mode/close styles; removed the retired `.top-bar-mode` rules; empty top-bar centre. |
| `src/styles/chat.css` | Centred `.chat-panel-frame`, empty `.chat-panel-center` stage, `.chat-panel-empty-label`, docked composer frame, `Show drawing` button. |
| `src/styles/drafting.css` | Container query for the HLR caption in a narrow drawing panel. |
| `test/AppShellChrome.dom.test.tsx` | **Rewritten** for the chat-first frame; adds default boot, primitive `Drawing` toggle, auto-open on `design/received`, close/unmount, `Ctrl+B`, and `Show drawing`. |
| `test/TopBar.dom.test.tsx` | **Rewritten** for the wordmark-only left, empty centre, `Drawing` toggle; no render mode / no chat toggle. |
| `test/DrawingPanel.dom.test.tsx` | **New.** Header name, single render-mode home, one canvas, close dispatch. |
| `test/layoutCss.test.ts` | App-frame invariants rewritten: chat alone, split tracks, 760px frame, empty stage, drawing header, 1279/1100 media rules. |
| `test/AgentWorkspace.dom.test.tsx` | Boot/readout/title-block/wiring tests open the drawing first; no-canvas-at-boot and primitive-toggle cases added; render mode read from the drawing header. |
| `test/AppBoot.dom.test.tsx` | Shell-reached assertions now use the top bar/chat; added drawing-closed/no-readout boot case. |
| `test/AppShellMeshStability.dom.test.tsx` | Opens the drawing before comparing the viewport mesh (the canvas only exists then). |
| `test/BottomBar.dom.test.tsx` | Opens the drawing in the readout suite; added readout-hidden-while-closed case. |
| `test/BottomBarDesignReadout.dom.test.tsx` | Opens the drawing before reading the bar. |
| `test/previewStatus.dom.test.tsx` | Bottom-bar preview verdict tests open the drawing first. |
| `test/ManualConnect.dom.test.tsx` | Asserts the chat-first shell with no sheet at boot; preview comment corrected. |
| `test/ChatPanel.dom.test.tsx` | Starter/label assertions moved to the centred empty stage. |
| `test/chatJournal.dom.test.tsx` | Scroll-pinning test seeds one reply so the thread exists. |
| `test/DesignAgentCard.dom.test.tsx` | Adds `Show drawing` and Section A–A → drawing + Details › Views coverage. |

No behaviour was dropped. The old top-bar render-mode test moved to the
drawing header (`DrawingPanel.dom.test.tsx`, plus the end-to-end wiring test
in `AgentWorkspace`), and the old chat-toggle tests were re-pointed at the
`Drawing` toggle (D3/D9).

## Gates actually run

All commands in `studio/` with `RUSTUP_HOME`, `CARGO_HOME`,
`npm_config_cache` and a throwaway `HOME` / `KALA_AGENT_DATA_DIR` /
`KALA_STUDIO_DATA_DIR` / `XDG_DATA_HOME`.

- `npx tsc -p tsconfig.json --noEmit` → **exit 0 (0 errors)**.
- `npx vitest run` → **59 files, 854 tests, all passed** (3.08 s).
- `npm run build` → **exit 0**, writes `studio/dist` (allowed for this cell).
- Visual build: `npx vite build --outDir /tmp/kala-chatfirst-dist --emptyOutDir`
  (exit 0), served with `tools/serve.sh 8797 /tmp/kala-chatfirst-dist`, shots
  with `node tools/shot.mjs` against the real native backend.

## Screenshots (`cells/chat-first/shots/`)

| File | Shows |
|---|---|
| `empty.png` (1440×900) | Boot, empty thread: `New design`, composer around the middle, starters under it, no drawing/canvas/readout. `shot.mjs` counted 13 visible controls. |
| `thread-no-drawing.png` (1440×900) | Local helper + OpenCascade L-bracket turn, then `Close drawing`: centred thread, docked composer, collapsed design card with `Show drawing`, no readout. |
| `design-split.png` (1440×900) | Right after the Local helper + OpenCascade `L-bracket 100 x 80 x 60 mm, 8 mm wall` turn: drawing opened automatically, 5/5 checks, `CAD: OpenCascade`. 27 visible controls. |
| `design-split-details.png` (1440×900) | Same turn + Details › Properties; the drawing panel is ~610px, HLR caption compacted to title + cut note. |
| `design-split-1280x800.png` (1280×800) | Same turn at 1280×800; chat keeps its 360px minimum, drawing 1.3fr. |
| `slate-design-split.png` (1440×900) | Same turn in the Slate theme. |
| `3d-clay.png` (1440×900) | Same turn after `3D Clay` in the drawing header; clay viewport fills the drawing panel. |

The 1100px Details-overlay case was also checked ad hoc at 1000×800 in the
same build: Details overlaid the right side of the drawing column, bounded to
the content row, with no fourth track. The empty-state model popover was
checked at 1440×900 and 1280×800 and was not clipped by the centred stage.

## Left over / notes

- The chat paper surface still spans the content area; the 760px frame
  constrains and centres every band of content. This keeps the existing
  panel/paper look while reading as one centred conversation.
- The HLR caption's container query hides only the secondary
  `meta`/`view` lines and the decorative `HLR VECTOR …` footnote below a
  760px drawing panel; the drawing title and redline cut note remain. This is
  a responsive rule, not a new colour, font, radius or motif.
- `tokens.css` was not touched. The retired `--zone-*` / `--bottom-bar-height`
  tokens remain defined in it (as noted by the integrate cell); no
  stylesheet consumes them.
- Details tab-strip arrow-key navigation is still not implemented (pre-existing,
  not part of this task).
- The 8797 server and every `/tmp` build/look directory created by this cell
  were stopped and removed after the screenshots.
