# UI cell report — Chat-First Mechanical Engineering Agent + OpenCascade Workbench

## Status

Complete and verified. `npm test` → **156 tests across 14 files, all passing**
(was 125/13; +31 new tests, **zero regressions** in the 13 pre-existing files).
`npm run typecheck` → 0 errors. `npm run build` → clean Vite production bundle
in `studio/dist`.

## What changed

### 1. Agent owns the primary viewport — `studio/src/components/shell/AppShell.tsx`

`ChatPanel` now renders inside `.app-shell-viewport` (`grid-area: viewport`) on
boot; `SceneCanvas` only takes that band when the operator explicitly switches
to 3D focus mode. The rail moved from third-place accessory to first-class
workbench: it receives the resolved `mesh`, `meshSource`, and `viewport`
preferences directly from the shell, so both canvas placements render the same
server-verified-or-local mesh.

The view mode is component state (`useState`), deliberately **not** store state:
it is a view preference with no bearing on geometry, so `previewController` can
never be woken by flipping the view.

### 2. Workspace view switch — `studio/src/components/toolbar/Toolbar.tsx`

A `role="radiogroup"` switch (`Agent + Workbench` / `3D Canvas Focus`) placed at
the head of the existing toolbar, each button carrying `data-view-mode` plus
`aria-checked` and a descriptive `title`.

It lives inside the `toolbar` grid area **on purpose**: `.app-shell-main` must
keep `grid-template-rows: auto minmax(0, 1fr)` (pinned by
`layoutCss.test.ts`), so a third grid row was not an option. The view switch is
the view bar the task asked for, placed in the only region that can hold it
without breaking the pinned layout invariant.

### 3. `studio/src/components/shell/workspaceView.ts` (new)

`WorkspaceViewMode` (`'agent' | 'canvas'`), `DEFAULT_VIEW_MODE = 'agent'`, and
`VIEW_MODE_OPTIONS` with per-option descriptions. One shared definition so the
toolbar and the rail can never disagree about what "focus" means.

### 4. Rail is the OpenCascade Geometry & CAD Workbench — `studio/src/components/shell/RightRail.tsx`

- Tabs are now `Workbench` (default) · `Export` · `Assistant`, `aria-label`
  "OpenCascade geometry and CAD workbench". The `Inspector` / `Export` /
  `Assistant` panel set is unchanged, so any code path that reaches those
  components keeps working.
- A docked `.right-rail-preview` hosts a live `SceneCanvas` whenever the main
  viewport is showing the agent.

**WebGL discipline (single context):** `docked = viewMode === 'agent'`. In agent
mode the rail mounts the one `SceneCanvas` and the viewport has none; in focus
mode the viewport mounts it and the rail drops its preview. They are mutually
exclusive by construction, not by convention — asserted directly in
`AgentWorkspace.dom.test.tsx` by counting `.viewer-canvas` hosts across three
transitions.

### 5. Nominal geometry estimates — `studio/src/viewer/localPrimitives.ts`

New pure export, no DOM/network/store dependency:

```ts
interface GeometryEstimate {
  envelopeMm: [number, number, number];   // ΔX, ΔY, ΔZ
  envelopeVolumeMm3: number;              // ΔX · ΔY · ΔZ
  volumeMm3: number;  volumeCm3: number; // mm³, /1000
  surfaceAreaMm2: number; surfaceAreaCm2: number; // mm², /100
}
computeGeometryEstimate(kind: PrimitiveKind, p: StudioParameters): GeometryEstimate
```

Box `V = XYZ`, `A = 2(XY + YZ + ZX)`; Cylinder `V = πr²h`, `A = 2πr(r + h)`;
Sphere `V = ⁴⁄₃πr³`, `A = 4πr²`. Estimates are **independent of tessellation**
(segment counts change no nominal value) and are pure/deterministic.

Two decisions worth flagging:

- **Magnitudes, not signed values.** `Math.abs` is applied to the driving
  dimensions so an already-invalid negative input still yields a well-formed
  extent instead of a negative volume. Range rejection stays where it belongs —
  `validateParameters`. There is a test pinning both halves of that contract.
- **Non-finite degrades to `—`**, never `NaN`, via the four exported
  formatters (`formatVolumeEstimate`, `formatAreaEstimate`,
  `formatEnvelopeEstimate`, `formatEnvelopeVolumeEstimate`), shared by the
  inspector and the chat banner so the two can never print different numbers.

### 6. Inspector estimate panel — `studio/src/components/inspector/Inspector.tsx`

A `.inspector-estimates` section between the parameter form and `BoundsReadout`
shows Volume (mm³/cm³), Surface area (mm²/cm²), Bounding envelope (ΔX × ΔY × ΔZ
mm), and envelope volume, with an explicit note: *"Closed-form nominal geometry
for the ideal solid, for sizing, mass estimation, and STEP/STL handoff to an
external CAD/CAE package. No stress, deflection, or tolerance results are
computed here."* It renders unconditionally (not gated on a local preview) so
the numbers are there before the first mesh exists.

### 7. Agent framing — `studio/src/components/chat/ChatPanel.tsx`

- Header is now **"Mechanical Engineering Agent"** with the subtitle
  *"Conversational mechanical sizing, parametric geometry estimation, and
  STEP/STL workflow planning."*
- **Active Geometry Context** banner (`.chat-panel-context`): live primitive
  (`box`/`cylinder`/`sphere`), driving dimensions in mm, bounding envelope, and
  nominal volume — all derived from the same store slice the workbench reads,
  so chat and workbench cannot drift.
- **Engineering Starter Prompts** (`.chat-panel-starter`, `data-starter`):
  *Size a bracket*, *Size a shaft*, *Estimate a housing*, *Plan a STEP/STL
  handoff*. They **seed the composer and focus it — they never auto-send**, so
  a stray click cannot spend a remote-provider call. A test asserts `fetch` is
  untouched after a starter click.

**Selector contract preserved and extended.** Every documented selector is
intact; the ones the task named explicitly are now present:

| Selector | Status |
|---|---|
| `#chat-provider-select` (`select`) | added id |
| `#chat-model-input` (`input[type=text]`) | added id |
| `input[type=checkbox]` (`allowRemote`) | unchanged |
| `textarea.chat-panel-prompt` | added class + placeholder *"Ask a short question about the current primitive…"* |
| `button.chat-panel-send` / `-cancel` / `-clear` | unchanged (`Send` / `Cancel` / `Clear`) |
| `.chat-panel-message-user` / `-assistant` / `-error` / `-empty` | unchanged |

Ordering matters for the existing suite (it grabs the *first* `select` and
*first* `textarea`): no starter or control is a `select`/`textarea`, and both
the new ids and an explicit ordering assertion in `AgentWorkspace.dom.test.tsx`
pin it.

### 8. Non-discrimination — enforced

No "hobbyist/pro", "amateur/professional", or equivalent split exists anywhere
in labels, state, tooltips, comments, or tests. One unified engineering
workflow is presented: the same OpenCascade workbench — 3D preview, parametric
sizing, nominal estimates, STEP/STL export — serves both a complete standalone
build and a quick estimate handed to an external CAD/CAE package. Copy updated
in `Sidebar.tsx`, `ExportPanel.tsx`, and `Header.tsx`
("chat-first mechanical engineering agent") to say so.

## CSS — invariants held, nothing pinned was touched

All 8 `layoutCss.test.ts` structural assertions pass unmodified, as do the CSP
assertions. `.app-shell-main` still `auto minmax(0, 1fr)`; the ≤1100px block
still `auto minmax(0, 1.4fr) minmax(0, 1fr)` with `"viewport"`/`"rail"` and no
`footer-area`.

Additions: `.chat-panel-intro`, `.chat-panel-context*`, `.chat-panel-starter*`,
`.inspector-estimates*`, `.right-rail-preview`, `.right-rail.has-preview`,
`.viewport-toolbar-view-switch`, `.app-shell-viewport > .chat-panel`.

One ordering trap worth recording: `cssBlock()` searches for `"<selector> {"`,
so `.app-shell-viewport > .chat-panel` **contains** the substring `.chat-panel {`
and was shadowing the base `.chat-panel` rule. The height override was moved
below the base rule so both selectors resolve unambiguously.

`.chat-panel` gained a row (`auto auto minmax(0, 1fr) auto`) for the intro
band; the thread is still the `minmax(0, 1fr)` scroller, so a long conversation
cannot push the composer out of the band.

## Tests added

`studio/test/AgentWorkspace.dom.test.tsx` (new, 12 tests) drives the **real**
`App` through the real boot path:

- agent in the primary viewport on boot, `data-view-mode="agent"`, no canvas there;
- rail labelled/structured as the workbench, docked preview present, **exactly one** `SceneCanvas`;
- `Workbench` / `Export` / `Assistant` tabs, default `Workbench`;
- focus toggle moves the canvas and keeps the count at 1 across both directions;
- view mode conveyed by `aria-checked`, not colour;
- estimates render exact numbers (box `24000.00 mm³ (24.000 cm³)`, `5200.00 mm² (52.000 cm²)`, `40.00 × 20.00 × 30.00 mm`) and carry the nominal/no-FEA note;
- estimates **and** the agent banner both follow a Box→Cylinder switch (π values to 2 dp) — proves chat and workbench stay in lockstep;
- starter prompt seeds the composer without sending;
- documented composer selectors present, correct ordering;
- remote provider stays locked until model **and** consent are both supplied, with no `fetch`.

`studio/test/localPrimitives.test.ts` (+19 tests): closed-form values per
primitive, tessellation-independence, mm³→cm³ / mm²→cm² consistency across all
three, envelope-volume-vs-solid-volume separation, purity/determinism,
magnitude-not-signed behaviour paired with validator rejection, and formatter
units plus `NaN → —`.

`studio/test/layoutCss.test.ts` (+6 tests): agent fills the band, bounded thread
row, capped intro band, rail preview is positioned/bounded/clipping, conditional
third rail row, estimate panel legibility.

## Files touched (all in scope)

Modified: `src/viewer/localPrimitives.ts`,
`src/components/inspector/Inspector.tsx`, `src/components/chat/ChatPanel.tsx`,
`src/components/shell/AppShell.tsx`, `src/components/shell/RightRail.tsx`,
`src/components/shell/Header.tsx`, `src/components/sidebar/Sidebar.tsx`,
`src/components/toolbar/Toolbar.tsx`, `src/components/export/ExportPanel.tsx`,
`src/styles/studio.css`, `test/localPrimitives.test.ts`, `test/layoutCss.test.ts`.
Added: `src/components/shell/workspaceView.ts`, `test/AgentWorkspace.dom.test.tsx`.

No store, API-client, types, Electron, or backend file was modified — the
geometry estimate is a pure viewer-layer addition and the view mode is local UI
state, so the backend contract is untouched. Nothing outside
`studio/src/**`, `studio/test/**`, and this report was written. Nothing
committed, pushed, or deployed.

## Verification (from `studio/`)

| Command | Result |
|---|---|
| `npm test` | 14 files, **156/156 passing** (125 pre-existing + 31 new) |
| `npm run typecheck` | clean, 0 errors |
| `npm run build` | `✓ built in 168ms`, all 5 chunks emitted, new classes present in `dist/assets/index-*.css` |

The `chunks are larger than 500 kB` notice from Vite is pre-existing and comes
from the `three` vendor chunk; it is unrelated to this pivot.

## Notes for the conductor

- **Not verified in a real browser.** No display/WebGL session was used here;
  verification is the automated suite plus a production build. The layout
  reasoning above (two-row main grid, `1fr` thread, capped intro, bounded rail
  preview) is what keeps 1280×800 bounded, but a human eye on the real window is
  still worth one pass.
- The `Assistant` rail tab remains a secondary view of the same agent thread.
  With the agent now in the viewport this is redundancy by design (one click
  away), not the primary path.
- `studio/README.md` is outside this cell's edit scope and still describes the
  pre-pivot arrangement; whoever owns docs should refresh the workspace layout
  description.