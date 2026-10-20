# Kala Studio — the Vellum drafting board (desktop UI)

> The renderer for the chat-first Mechanical Engineering Agent: a
> four-zone drafting workspace with a real CAD-provider model, a kernel
> drawing, a section cut and a hash-chained session ledger.
>
> This package owns the **renderer** only. The back door (the native Rust
> `kala-studio` service, with the real OpenCascade behind it), the CAD
> providers and the packaging live elsewhere in this repository.

## Quick reference

```bash
npm install              # install UI dependencies
npm run typecheck        # TypeScript-only build check
npm test                 # 43 test files, 628 tests (at the time of writing)
npm run build            # build production UI bundle into dist/
npm run desktop          # dev launcher: builds UI if needed, starts backend, opens Electron shell
npm run desktop:pack     # verify UI + staged native backend present, then run electron-builder
```

Two more commands matter for the Rust front door:

```bash
npm run check:native-readiness   # route-surface parity report (--report-only)
npm run build:native             # stage the native runtime + dist + the OCCT libraries
```

## The back door is Rust-only

Since the cutover (ADR-0010 phase 2, 2026-10-20) the packaged app starts
**only** the native Rust `kala-studio` binary:

* The packaged Electron main process resolves `resources/backend/kala-studio`
  (or `.exe`) **only**. It never consults `KALA_STUDIO_PYTHON`,
  `KALA_STUDIO_DEV_BACKEND`, a PATH Python or a repository virtualenv, and
  it removes `PYTHONPATH`, `PYTHONHOME`, `KALA_STUDIO_PYTHON`,
  `KALA_STUDIO_DEV_BACKEND` and `KALA_STUDIO_GEOMETRY_WORKER` from the
  child environment.
* In **development** the staged native slice is the default too. The Python
  reference is reachable only behind an explicit
  `KALA_STUDIO_DEV_BACKEND=python` (or a `KALA_STUDIO_PYTHON` override).
* The only Python inside the package is the **FreeCAD adapter**, and it runs
  inside the user's own FreeCAD, which is **not** bundled. Its runtime
  dependencies are vendored per CPython ABI (`site-packages-cp311` …
  `site-packages-cp314`), so the installed app needs no virtualenv and no
  `PYTHONPATH`. The `mod/Kala` workbench addon is staged, not installed.
* The geometry comes from the **real OpenCascade 7.9.3** through the
  `kala-cad` C++ adapter; the 23 `libTK*.so*` libraries are staged with
  `RUNPATH=$ORIGIN/lib` on Linux, and the `TK*.dll` closure next to
  `kala-studio.exe` on Windows.

The Linux deliverable is the **unsigned** AppImage
`studio/release/kala-studio-0.1.0-x86_64.AppImage` (163 642 516 bytes,
sha256 `b069b7d4b73f8ae01b166744b61f090455ef989ce348f360826ccfd69aaba31e`).
The Windows NSIS build script exists but **has never been executed**. See
[../docs/studio-install.md](../docs/studio-install.md).

## The four zones

```
┌──────────────────┬───────────────────────────────────┬──────────────────┐
│ agent chat 340px │  Vellum drafting sheet (flex)     │ AuxView  320px   │
│                  │  zone ticks · 50px grid · model   │ ─────────────────│
│  CAD-target      │  redline dimensions · title block │ Agent & Kernel   │
│  selector        │  tool palette · sheet tabs        │ Journal (ledger) │
│                  ├───────────────────────────────────┤                  │
│                  │  bottom bar 48px: mode · toggles  │                  │
│                  │  readout · Export STEP · Options   │                  │
└──────────────────┴───────────────────────────────────┴──────────────────┘
```

There is no top header bar and no application footer. The connection
information lives in the journal footer and in the Options drawer. The
columns are `340 / 780 / 320` at 1440×900 and `280 / 740 / 260` at
1280×800. There is exactly **one** WebGL canvas in the app and it
survives mode switches, theme switches and drawer opens.

## The components

| Area | Files | What it does |
|---|---|---|
| **Shell** | `src/components/shell/AppShell.tsx`, `BootGate.tsx` | the four zones, the connection gate, the theme bootstrap |
| **Chat** | `src/components/chat/ChatPanel.tsx`, `CadTargetSelector.tsx`, `DesignAgentCard.tsx`, `designCard.ts` | the thread, the composer, the **CAD-target selector**, the design result card |
| **Drafting sheet** | `src/components/drafting/DraftingSheet.tsx`, `AuxView.tsx`, `useDesignController.ts` | the sheet, the title block, the tool palette, the Section/Ortho/Tolerance tabs |
| **Journal** | `src/components/journal/JournalPanel.tsx`, `ledgerRows.ts`, `CopyButton.tsx` | the six-column ledger, the chain stamp, the digest copy |
| **Bottom bar** | `src/components/bottombar/BottomBar.tsx` | the render-mode switch, the toggles, the live readout, Export STEP, Options |
| **Options drawer** | `src/components/options/OptionsDrawer.tsx` | the four drafted cards (Inspector / Export & Handoff / Providers / Connection) |
| **Viewport** | `src/viewer/SceneCanvas.tsx`, `HlrDrawing.tsx`, `measure.ts`, `raycast.ts`, `draftingGeometry.ts`, `designSource.ts` | the WebGL scene, the pencil HLR sheet, the measure tool |
| **Data** | `src/api/client.ts`, `src/api/validators.ts`, `src/state/**`, `src/types/studio.ts` | the typed client, the payload validators, the store |

## The rules that keep it honest

These are invariants, not preferences. A test enforces each of them.

1. **No fake data.** Every number, status, endpoint, receipt and journal
   row comes from the live store, the live backend answer, or an explicit
   empty state (`—`, `not provided`, `NO LOOPBACK`, `STEP —`,
   `unavailable`). Nothing is invented to look complete.
2. **`B-Rep Verified` is guarded by `design.verified`.** It is printed only
   when every check passed. A `not_reported` check is neither a pass nor a
   failure, and it prints a `—` mark.
3. **`HASH-CHAIN OK` is guarded by a real `/journal/verify`.** The store
   becomes `verified` only from a `ok: true` answer. A local row, a page,
   or a poll failure never lights the stamp. A broken chain prints
   `CHAIN BROKEN @ n`; no verification at all prints `SESSION LOG`.
4. **The measurer is named.** The card and the bottom bar say
   `measured by <provider> <version>`, taken from `measured_by`. The UI
   never labels OpenCascade as the measurer of a FreeCAD or Solid Edge
   part. The viewer label is a separate statement and always names the
   OpenCascade **import** of the provider's STEP.
5. **A missing number is `—`, never a substitute.** Every nullable
   `measured` field and every nullable `checks[].measured` renders as an
   em dash.
6. **There is no default CAD provider.** `state.cadProvider` starts
   `null`, is never written to `localStorage`, and a turn with no target
   builds nothing.
7. **Only the theme name is persisted**, in
   `localStorage['kala-studio.theme']`. No origin, no token, no chat text,
   no journal content.
8. **The journal is bounded** at 500 rows; the oldest is dropped first.
   The journal poller reads bounded pages (1 s busy, 5 s idle, paused
   while the document is hidden) and backs off exponentially on failure.
9. **The security posture is unchanged**: `index.html`'s CSP, the Electron
   main process, the preload, the single `getConnection` IPC channel, the
   in-memory token and the loopback-only origin are exactly as they were.

## The render modes

* **2D Pencil** — vector drafting. With a design active it draws the
  kernel's hidden-line result (visible solid, hidden dashed) as SVG over
  the sheet.
* **3D Clay** — a warm clay solid in the live WebGL viewport, rendering
  the **imported** provider STEP.

Switching modes re-frames the camera (the field of view changes).
Changing the theme, the material, or the grid/axes/dimension toggles
does **not** re-frame and does not rebuild the GPU buffers — the mesh
object handed to the viewport is referentially stable.

## The measure tool

The fifth tool in the sheet palette, disabled while no mesh exists. The
pick is a real Three.js raycast against the face mesh, so the distance is
a real length in model millimetres. A pointer that moved more than 4 px
between press and release was an orbit drag and is ignored; a click that
misses the mesh changes nothing. `Escape` or a second click on the tool
button clears the measurement; a third pick starts a new one.

## The themes

`vellum` (day) and `slate` (night). The palette is token-driven
(`src/styles/tokens.css`); the canvas background follows the theme so the
viewport surface and the sheet behind it cannot disagree about the paper
they are drawn on. The contrast pass in the vellum theme raised the
construction grey and introduced a separate `--redline-ink` token so the
red **line** colour (pinned by a test) is not the red **text** colour.

## Build outputs

`npm run build` writes a production bundle to `dist/` with chunk-split
React and Three.js files. Font files are **never** inlined as `data:`
URIs — `vite.config.ts` excludes them so the served CSP (`font-src
'self'`) is satisfied without changing either CSP. 15 same-origin
`.woff2` files are emitted.

The Electron shell loads `http://127.0.0.1:<port>/` where `<port>` is
the free loopback port the back door bound to; the back door serves
`dist/` from the same origin as the API.

`npm run build:native` stages `backend-native/`:

```
backend-native/
  kala-studio                 the Rust release runtime
  studio_dist/                the Vite UI served by the backend
  native-build.json           digests, source parity, staging metadata
  lib/                        the exact ldd-resolved OCCT libTK*.so*
  adapters/freecad/
    vendor.json               supported ABIs, wheel digests, detected FreeCAD
    src/kala_agent_freecad/   the adapter modules for FreeCADCmd
    src/kala_agent/           the shared pure-Python dependency
    site-packages-cp311/ …    the vendored wheels per CPython ABI
    mod/Kala/                 the FreeCAD workbench addon
```

electron-builder copies that directory to `resources/backend/` unchanged.

## Honest limitations

- This package does **not** run native CAD or analysis tools. It only
  produces neutral-format files for handoff and surfaces the registry the
  back door reported. If the registry marks an integration as native-only,
  the UI says so and does not pretend otherwise.
- The WebGL viewport is a triangle-mesh renderer, not a B-rep kernel. STEP
  geometry is consumed through the back door's importer, but the in-app
  viewport always shows a tessellation.
- The clay viewport uses a **narrow-FOV perspective camera**, not an
  orthographic one. See
  [../docs/studio-limitations.md](../docs/studio-limitations.md).
- The desktop build runs **unsigned**. The Linux AppImage exists and is
  smoke-tested, but it is not signed and not published; the Windows NSIS
  build script has **never been executed**, so no Windows runtime is
  claimed. See
  [../docs/studio-install.md](../docs/studio-install.md).
- The Electron **window** was never rendered on the build host (`xvfb-run`
  is absent): the packaged backend and the served UI root are verified by
  extracting the AppImage, not by driving the Chromium shell.
- The `three` chunk is over 500 kB and Vite warns about it. No
  code-splitting change has been made.
