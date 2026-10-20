# REPORT — cell `c0-gaps` (wave `vellum-ui-20261019`)

UI worker C0 "gaps": the seven backend-independent gap items triaged from
`cells/audit-minimax/AUDIT.md`. No commit, no push, no delegation, no
credential read, no Python touched. Owned files: `studio/src/**`,
`studio/test/**`, plus this cell folder.

Status: **complete** — all seven items are built, gated, and verified in a
real browser against the real loopback backend.

---

## 1. Files changed

### New — `studio/src/**`

| File | What it holds |
| --- | --- |
| `viewer/measure.ts` | The measure tool as a pure module: the distance maths, the tool state machine, the status text. No DOM, no Three.js. |
| `viewer/raycast.ts` | The only file that knows about the renderer: `pointerToNdc` and `raycastFacePoint` (a real Three.js raycast against the face mesh). |
| `components/panels/SurfaceMaterialControl.tsx` | The one control that writes `viewport.material`; a `compact` variant for the ruler bar and a `full` variant for Options card 01. |

### Modified — `studio/src/**`

| File | Change |
| --- | --- |
| `viewer/draftingGeometry.ts` | `DRAFTING_SCALE` (the honest `FIT`), `computeCentreLineAxes(bounds, pad)`, `chainDashSegments(from, to, pattern)` (the — · — chain-dash). |
| `viewer/SceneCanvas.tsx` | The measure pick (pointerdown/pointerup → raycast), the redline measure leader + its projected label, the drafted chain-dash centre lines + the `+X (WIDTH)` / `+Y (HEIGHT)` labels, and one shared overlay-label projection for all three label groups. |
| `state/store.ts` | `UiState.measure` + `measure/toggle`, `measure/point`, `measure/clear`; the chat message metadata (provider / model / offline / durationMs). |
| `components/drafting/AuxView.tsx` | The section is now cut and labelled on the **real mesh bounds**; the redline `CUT: 45° ANSI` note moved into the aux **header**; the footer is one accurate line. |
| `components/drafting/DraftingSheet.tsx` | The measure tool button (disabled without a mesh), the measure state note, the Escape handler, and the wiring into the viewport. |
| `components/bottombar/BottomBar.tsx` | The `SCALE` readout and the compact surface-material control. |
| `components/inspector/Inspector.tsx` | Options card 01 carries the full surface-material control and a note that the canvas background follows the theme. |
| `components/chat/ChatPanel.tsx` | The agent card badge (provider · model, or `local · offline`) and the client-measured reply time. |
| `components/shell/AppShell.tsx` | One line: the title-block `SCALE` cell now reads `DRAFTING_SCALE`, so the sheet cell and the ruler bar cannot drift apart. |
| `styles/layout.css` | The centre readout wraps onto a second line instead of clipping a value; the scale item never shrinks; the compact material control is hidden at ≤1400px. |
| `styles/drafting.css` | The tool-palette wrapper, the measure note, the measure/centre overlay labels, the measuring cursor, the aux header + cut note. |
| `styles/studio.css` | The surface-material control and the Options card 01 material block. |
| `styles/chat.css` | The agent badge and the elapsed chip. |

Nothing outside `studio/src/**`, `studio/test/**` and this cell folder was
touched. `studio/index.html` (CSP), `studio/electron/**`, `studio/scripts/**`,
`electron-builder.json` and every file under `src/kala_agent/**` are
untouched by this cell.

---

## 2. The contract this cell adds

### Store (`studio/src/state/store.ts`)

```ts
export interface MeasureState {
  active: boolean;                 // the tool is armed
  points: MeasurePoint[];          // picked points, at most two, in model mm
}
export interface MeasurePoint { x: number; y: number; z: number }

UiState.measure: MeasureState;     // default { active: false, points: [] }

| { type: 'measure/toggle' }        // arm/disarm; arming or disarming drops stale picks
| { type: 'measure/point'; point: MeasurePoint }
| { type: 'measure/clear' }         // disarm + drop the picks
```

`parameters/change`, `parameters/reset` and `primitive/change` clear the
picked points (they belong to the old solid) but keep the tool armed.

```ts
export interface ChatMessage {
  role: 'user' | 'assistant';
  text: string;
  at: number;
  provider?: ProviderId;   // the provider the backend answered with
  model?: string | null;   // the model the backend answered with
  offline?: boolean;       // the backend answered from the offline local helper
  durationMs?: number;     // client-measured send-to-reply time
}
| { type: 'chat/response'; response: { text; provider; model; offline?; durationMs? } }
```

`offline` and `durationMs` are optional, so the existing two-argument
`chat/response` call sites keep compiling and keep their behaviour.

### `SceneCanvasProps` (new members)

```ts
measure: MeasureState;
onMeasurePoint(point: MeasurePoint | null): void;   // null = the pick missed the mesh
```

`DraftingSheetProps` is unchanged.

### New pure exports

```ts
// viewer/measure.ts
distanceMm(a, b): number
toggleMeasure(state), addMeasurePoint(state, point), clearMeasure()
measureDistanceMm(state): number | null
measureStatus(state), measureStatusLabel(state)
formatMeasureDistance(mm): string            // "40.00 mm"

// viewer/draftingGeometry.ts
DRAFTING_SCALE: 'FIT'
computeCentreLineAxes(bounds, pad): CentreLineAxis[]
chainDashSegments(from, to, pattern): Array<[from, to]>

// viewer/raycast.ts
pointerToNdc(clientX, clientY, rect): THREE.Vector2
raycastFacePoint(camera, target, ndc): MeasurePoint | null
```

---

## 3. The seven items

### 1. AuxView section label uses the real bounds — **done**

`AuxView.tsx` now calls `computeSectionView(props.primitive, props.parameters,
props.bounds)`. The follow-up named in `cells/fix-camera/REPORT.md` § 5.1 is
closed. For every solid this studio builds the behaviour is identical (they
are all centred on the origin); for a mesh that is not centred the aux panel
now names the same plane the viewport cuts
(`Z = 15.00 mm · mid-plane` for bounds `z ∈ [0, 30]`).

### 2. SCALE readout in the bottom-bar centre cluster — **done**

`SCALE FIT` is the first item of the centre cluster. `FIT` is the honest
value: the viewport frames the part to fit the sheet (`frameOnBounds`), so
there is no fixed model-to-paper ratio and `1:1` would be a false claim.
The value comes from one constant (`DRAFTING_SCALE`) that the sheet title
block also reads, so the two can never disagree.

**Measurement first, then the layout fix.** The ruler bar was already full at
1440 px (centre column 780 px): left cluster 237 px + right cluster 244 px
leaves 271 px for a readout that needs 322 px. Adding `SCALE` pushed the
readout into ellipsis — the first screenshot showed `SCALE ENV 4… | VOL 2…`.
The readout now **wraps onto a second line** inside the same 48 px bar
(`flex-wrap: wrap`), the scale item is `flex: 0 0 auto` so it is never
shortened, and the readout type drops from 8.5 px to 8 px. Every value is
now fully legible at 1440 and at 1280.

### 3. Aux header carries the redline `CUT: 45° ANSI` — **done**

The note is now a redline element in the aux header, to the right of the
tab pills, exactly as in the Stitch (`code.html:1353-1365`). The footer lost
the duplicated note and keeps the accurate plane
(`PLANE: Z = 0.00 mm · mid-plane`). The ortho and tolerance footers are
unchanged (`PROJ: 3RD ANGLE | BOUNDS: LIVE`, `SOURCE: — | VALUES: NONE`).

### 4. Material control — **done, in two places**

`viewport.material` (`default` / `matte` / `wireframe`) now has a real UI:
a segmented control (`role="radiogroup"`, three `role="radio"` buttons with
`aria-checked`) in the bottom-bar left cluster, and the same control with
spelled-out labels on **Options card 01**. Both write the one store field;
the renderer already implemented all three materials.

**Why the bar hides it at 1280 px.** The task allows the drawer's card 01
"if the bar is crowded at 1280px". Measured in the real browser: at 1440 the
centre column is 780 px and at 1280 it is 740 px. With the material control
in the bar the readout would have 182 px at 1440 and 166 px at 1280, while
the content needs ~300 px — the values would be clipped. So the compact
control is `display: none` below 1400 px (the 1280 breakpoint) and Options
card 01 carries it there. The control is never unreachable.

**The background follows the theme, on purpose.** `viewport.background` is
*not* exposed as a control: `AppShell` derives the canvas background from
the workspace theme (vellum paper by day, night slate at night) so the
viewport surface and the drafting sheet behind it can never disagree about
the paper they are drawn on. This is stated in the control's header comment
and in the Options card 01 note ("The canvas background follows the
workspace theme (paper by day, slate at night) and is not a separate
setting.").

### 5. Measure tool — **done**

A fifth tool in the sheet palette (a caliper icon, `aria-label="Measure
distance"`, `aria-pressed`, **disabled while no mesh exists**).

* **Picking** is a real Three.js raycast against the face mesh. The point is
  in model millimetres, so the number is a real length. A pointer that
  moved more than 4 px between press and release was an orbit drag, not a
  pick, and is ignored. A click that misses the mesh reports `null` and
  changes nothing.
* **The leader** is a redline line between the two points with short jaws at
  both ends; the **label** carries the distance in mono (`20.10 mm`) and is
  projected from the same camera as the dimension labels, so it follows the
  orbit.
* **The sheet note** under the palette states what the tool is waiting for
  (`MEASURE · PICK POINT 1 ON THE MESH`, `MEASURE · PICK POINT 2 ON THE
  MESH`) or the measured distance (`MEASURE · 20.10 mm`). It never invents
  a value while the measurement is incomplete.
* **Clearing**: Escape (the sheet listens on `document` and stands down
  while the Options drawer is open, because the drawer owns Escape) or a
  second click on the tool button. A third pick starts a new measurement.

### 6. Drafted centre lines — **done**

With Axes on, two chain-dash (— · —) redline lines run through the part
centre on the bounds mid-plane, with the small `+X (WIDTH)` and
`+Y (HEIGHT)` labels in 2H construction grey, projected from the 3D axes
through the same camera as the dimension labels. The chain-dash is real
geometry (`chainDashSegments`), not a dashed material, so the long-dash /
dot / gap pattern is exactly the drafted one. The lines follow the Axes
toggle (verified in the browser: 2 labels with Axes on, 0 with Axes off).

### 7. Chat agent card — **done**

The agent card now carries a badge with what the backend actually answered
with — `openai · gpt-4o-mini`, or `local · offline` when the response has
`offline: true` — and the reply time measured **client-side** in the
browser from the moment the request left to the moment the answer arrived
(`(1.2 s)`, or `(14 ms)` for a reply that came back in under a tenth of a
second, so the chip never reads like a stuck value). The provider, model
and offline flag come from the real `ChatResponse`; the duration is a real
round-trip measurement, not a backend figure.

---

## 4. Tests added

**New files (37 tests)**

| File | Coverage |
| --- | --- |
| `test/measure.test.ts` (13) | The distance maths (axis, 3D, symmetry, zero, formatting) and the tool state machine (arming clears stale picks, picks ignored while disarmed, non-finite picks rejected, two points before a distance exists, a third pick starts a new measurement, `clear`, the status text). |
| `test/raycast.test.ts` (7) | `pointerToNdc` (centre, corners, zero-sized rect) and `raycastFacePoint` against a **real** Three.js camera and box geometry: the model-space hit point in millimetres, an off-centre hit, a miss, and no target. |
| `test/MeasureTool.dom.test.tsx` (6) | The sheet tool with the viewport mocked so the pick can be driven: disabled without a mesh, arming, the measured distance between two picked points, a missed pick, Escape, and the second tool click. |
| `test/BottomBar.dom.test.tsx` (3) | `SCALE FIT` in the readout (and never `1:1`), the nominal readout and its source badge, Reset View. |
| `test/SurfaceMaterial.dom.test.tsx` (4) | The three real material kinds, the store field written from the ruler bar, the same field written from Options card 01, and the background-follows-the-theme note. |
| `test/ChatAgentCard.dom.test.tsx` (4) | The `local · offline` badge and the measured reply time (a controlled clock makes 1200 ms exact), the provider · model badge, the millisecond format, and no badge on the operator turn. |

**Modified files (+12 tests)**

* `test/AuxView.dom.test.tsx` — the section label on real bounds; the
  `CUT: 45° ANSI` note in the header and the shorter footer.
* `test/DraftingSheet.dom.test.tsx` — the palette now holds five tools (the
  old assertion of four asserted the old palette structure); the measure
  tool is disabled without a mesh.
* `test/draftingGeometry.test.ts` — the centre-line axes through the part
  centre (including a mesh that is not centred on the origin), the chain-dash
  geometry, and `DRAFTING_SCALE === 'FIT'`.
* `test/store.test.ts` — the chat reply metadata; the measure state machine
  in the reducer, including the stale-pick drop on a geometry change.

No behavioural coverage was deleted.

---

## 5. Gates

Run in `/home/v9i/Projects/Kala-Agent/studio`:

| Gate | Command | Result |
| --- | --- | --- |
| Type check | `npm run typecheck` | **green** — no output, exit 0 |
| Tests | `npm test` | **green** — **27 test files passed, 305 tests passed** (0 failed) |
| Build | `npm run build` | **green** — `✓ built in 444ms`; only the pre-existing `three` chunk >500 kB warning |

Baseline before this cell: 21 files / 256 tests. After: 27 files / 305 tests
(**+6 files, +49 tests**). The `three` chunk-size warning is unchanged and
unrelated.

---

## 6. Screenshot

`shots/01-vellum-1440-measure.png` — **1440×900**, the measure tool showing a
real measurement.

Captured exactly like `cells/qa/REPORT.md`: the real `.venv` backend
(`python -m kala_agent.studio`, loopback `127.0.0.1:8765`) serving
`studio/dist`, connected through the normal browser-fallback no-persistence
form, in headless Chromium.

What it shows, all of it real:

* the measure tool armed, the redline leader drawn across the solid with the
  mono label `20.10 mm`, and the sheet note `MEASURE · 20.10 mm`;
* the drafted chain-dash centre lines with `+X (WIDTH)` and `+Y (HEIGHT)`;
* the agent card with the `local · offline` badge and the measured reply
  time from a real local-helper turn;
* the aux header with the redline `CUT: 45° ANSI`;
* the ruler bar with `SCALE FIT | ENV 40×20×30 mm` and
  `VOL 24.0 cm³ | AREA 52.0 cm² [server]`, and the material control in the
  left cluster.

Supporting shots: `shots/02-vellum-1280-measure.png` (1280×800, the
material control hidden in the bar as described above) and
`shots/03-slate-1440-measure.png` (the night theme).

**Browser evidence** (headless Chromium against the same backend):

| Check | Result |
| --- | --- |
| Console / page errors | none (0 errors, 0 failed requests) |
| Document overflow at 1440 and 1280 | 0 px |
| Centre lines follow the Axes toggle | 2 labels on → 0 off → 2 on again |
| Measure → Escape | measurement and note cleared |
| Measure → second tool click | measurement and note cleared |
| Pick that misses the mesh | no measurement created |
| Material control (bar) | `aria-checked` flips, the drawer control follows |
| Material control (Options card 01) | writes the same field; labels `Default / Matte / Wireframe` |
| Exactly one WebGL canvas | yes (unchanged) |

---

## 7. Invariant checks

| Check | Result |
| --- | --- |
| Fake values | `grep -rnE "28\.16\|14208\|8f3a\|1\.1\.3\|Llama-3\|B-Rep Verified\|ISO 2768\|WGS84\|KEYTAR\|FIPS" studio/src` → **no matches**. The scale says `FIT`, not `1:1`; the measure label is a real raycast distance; the reply time is a real client-side measurement. |
| Remote URLs in `dist/` | none (only the loopback literals and the three.js/React documentation strings) |
| `data:font` in `dist/assets/*.css` | 0 |
| CSP | `studio/index.html` and `src/kala_agent/studio/middleware.py` not touched by this cell; no CSP warning in the browser console. |
| `electron/**`, `scripts/**`, `electron-builder.json` | not touched by this cell. |
| Python (`src/kala_agent/**`) | not touched by this cell (a concurrent backend worker owns it). |

---

## 8. Known gaps / honest notes

1. **The bottom-bar centre readout now occupies two lines.** At the 780 px
   sheet width the five facts (scale, envelope, volume, area, source) do not
   fit on one 8 px line. They were not dropped and they are not clipped —
   they wrap inside the same 48 px bar. The Stitch's reference bar has a
   single-line centre cluster, so this is a visible deviation forced by the
   real content (the Stitch shows one envelope, one volume, and a fake
   verification stamp, while this build shows four live facts plus a
   provenance badge).
2. **The compact material control is hidden below 1400 px.** At 1280 the
   readout needs the space. Options card 01 carries the same control, so the
   setting is always reachable.
3. **The measure tool measures the drawn solid only.** A pick on empty
   space is ignored, so there is no "distance to nothing". The tool is
   disabled when no mesh exists.
4. **A third pick starts a new measurement** instead of being ignored. That
   is a deliberate convenience; the two rules the brief asked for (Escape
   and a second tool click) both clear the measurement.
5. **The centre lines are drawn on the bounds mid-plane** (the same plane
   the A–A cut uses) and extend 18 % of the part span past the geometry.
   The Stitch draws them in 2H grey; the brief asked for redline lines with
   2H grey labels, which is what is built.
6. **The measure label and the centre-line labels are drawn by the viewport**
   (imperative DOM inside the canvas overlay), not by React, so they are
   positioned by the same projection as the dimension labels and follow the
   orbit. The DOM test therefore mocks the viewport and drives the pick;
   the distance maths and the state machine are unit tested.
7. **Nothing fake or stubbed was added.** Every number in the new UI comes
   from the live mesh, the live store, the live backend response, or a
   client-side measurement of a real round trip. The only fixed strings are
   labels (`MEASURE · PICK POINT 1 ON THE MESH`, `CUT: 45° ANSI`,
   `+X (WIDTH)`, `Default / Matte / Wireframe`).

---

## 9. Deferred (not built here, by instruction)

The brief defers the items that need the new design API: the per-design
sheet tabs, the jump-to-section pill on the design results, the 6-column
ledger with digests, and the CLI import command. The Select / Mark / Eraser
tools, fake ratios, fake latency, and any change to `electron/`,
`index.html` CSP or packaging were out of scope and were not built.
