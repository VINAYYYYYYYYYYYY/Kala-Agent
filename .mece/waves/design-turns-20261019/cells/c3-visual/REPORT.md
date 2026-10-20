# C3 vision fixes — HLR labels, one Section A–A, server ledger, 1440 bar

Cell: `c3-visual`, wave `design-turns-20261019`.
Read: `TASK.md` including the Opus addendum, `../c2-provider-ui/REPORT.md`,
`../d1-qa/REPORT.md`, `../b2-providers/REPORT.md`, `../b3-freecad-adapter/REPORT.md`,
`../b4-solid-count/REPORT.md`, ADR-0009 and ADR-0011.

Status: **done** for `studio/src/**` and `studio/test/**`. No commit, push,
deployment, delegation, or credential read was performed. The backend used a
throwaway `KALA_STUDIO_TOKEN` from the launch environment only. Nothing outside
`studio/src/**` and `studio/test/**` was edited; the screenshots and this report
are under the cell directory.

---

## 1. The four fixes

### 1.1 HLR labels now name the backend's real view directions

`studio/src/viewer/hlr.ts` now exports `DESIGN_VIEW_BASES`, a TypeScript mirror
of `drawing.py::_VIEW_BASES`:

| view | viewer (part → eye) | horizontal | vertical | rendered caption |
|---|---|---|---|---|
| front | `(0, −1, 0)` | X | Z | `FRONT · LOOKING +Y` |
| top | `(0, 0, +1)` | X | Y | `TOP · LOOKING −Z` |
| right | `(+1, 0, 0)` | Y | Z | `RIGHT · LOOKING −X` |
| iso | `(+1, −1, +1)/√3` | `X+Y` | `X−Y+2Z` | `ISO · FROM (+X −Y +Z)` |

`designViewCaption(name)` derives the orthographic `LOOKING` text from
`−viewer` and the iso position text from `viewer`; `HlrDrawing` no longer has a
hand-written caption table. The third-angle placement (top above front, right of
front) is unchanged.

Before: `TOP · LOOKING −Y`, `FRONT · LOOKING −Z`, `RIGHT · LOOKING +X`, `ISO · HLR`.
After: the table above.

### 1.2 One Section A–A state for sheet, HLR redline, AuxView, and clay

New `studio/src/state/sectionCut.ts` owns the plane model:

```ts
interface SectionCut { plane: 'XY' | 'XZ' | 'YZ'; offsetMm: number | null }
```

* `DEFAULT_DESIGN_SECTION_CUT = { plane: 'XZ', offsetMm: 4 }` — ADR-0009
  criterion 4 (y = T/2 = 4 mm, through the upright and the base).
* `PRIMITIVE_SECTION_CUT = { plane: 'XY', offsetMm: null }` — the existing Z
  mid-plane for analytic primitives.
* `offsetMm: null` means the backend's documented mid-plane; the label resolves
  it from the measured bounds. It is not a fabricated zero.
* `sectionCutCorners(bounds, cut, pad)` is the one plane geometry used by the
  clay viewport.

The store now has `state.sectionCut` and `section/cut-plane`; `AuxView` reads and
writes it instead of a private `useState`. Changing the plane in the section
panel immediately changes the sheet label, the HLR redline, and the clay plane.
`AuxView` resets a newly active design to the XZ/4 default through the same
action.

Drawing:

* The measured `/section` loops are passed to `HlrDrawing`. In the coplanar view
  (XZ → front) the loops are drawn in true shape; in the iso view each loop is
  lifted to model space at `offset_mm`, projected with the backend's iso basis,
  and fitted with the same transform as the HLR lines. The trace is red 2H dash
  with `A` flags at its ends. No section response or a different plane draws no
  redline.
* The sheet-level horizontal `A` line is no longer drawn for an active design
  (it was the arbitrary mid-sheet line); the redline is now in the iso and front
  views where the plane is. Primitives keep the old sheet mark.
* The cut note moved from the top-right HTML overlay (which sat on
  `TOP · LOOKING …`) into the HLR heading, and the clay-mode note moved to the
  top-left. There is no sheet overlay over the top view at 1440 or 1280.

Before/after evidence:

| Measurement | Before | After |
|---|---|---|
| Sheet cut note | `CUT A–A · Z = 20.00 mm · mid-plane` | `CUT A–A · XZ · Y = 4.00 mm` |
| AuxView footer | `PLANE: XZ`, area 2,321.46 mm² | `CUT A–A · XZ · Y = 4.00 mm`, area 2,321.46 mm² |
| Cut label vs top caption | overlap: x 887.9–1096, y 70–83 vs x 878.6–946.5, y 70.5–79.5 | no overlap (label y 711–724, caption y 70) |
| HLR section traces | none | iso + front, `data-section-plane="XZ"` |
| Plane in clay | Z mid-plane (20 mm) | XZ at y = 4 mm |

### 1.3 Server journal events now appear as soon as a design lands

The B2 event shape was **not** the problem: the 10-event real page validates
through `parseJournalPage`, dedupes by sequence, and merges correctly (new test
uses the recorded page). The `after` cursor was also correct (`0` then `10`).

The actual failure was a poll-cadence race. The design is received **after** the
backend has written its 10 events, but the poller may have just finished a page
read (empty, during the build) and then run `/journal/verify`; that lit
`HASH-CHAIN OK` while the page containing `design.intent`,
`provider.build.*`, `artifact.stored`, `view.imported`, `design.measured`,
`design.checked`, and `chat.turn` was still waiting for the 5 s idle tick. The
before capture reproduces Opus's report exactly: 7 rows, all `local`, all
`SEQ —`, stamp `HASH-CHAIN OK`. 6.5 s later the same page showed all server rows.

Fix:

* `JournalPoller.wake()` schedules an immediate page read; if a read is in
  flight it sets a flag that is honoured as soon as that tick finishes.
* `App` wakes the poller whenever the newest design id changes, so the design
  events are fetched in the same render turn that adds the agent card.
* The reducer already merged pages by sequence and sorted by time; no change was
  needed there.

After: the immediate capture has **17 rows = 6 local + 10 server + 1 local**,
server `SEQ 1–10`, 64-char digest, `CHAIN ok`, interleaved by timestamp. The
local rows keep `SEQ —` / `CHAIN local`.

### 1.4 Bottom bar fits at 1440

* Source chip text is the short provider name (`FreeCAD`, `OpenCascade`,
  `Solid Edge`); the full `measured by FreeCAD 1.1.4` is in the title tooltip
  and the agent card.
* ENV stays `60×45×40 mm`; VOL/AREA round to whole units with thin grouping
  spaces (`35 571 mm³`, `11 889 mm²`). The exact numbers
  (`35571.416047 mm³`, `11889.101731 mm²`) are in the item titles and the card.
* `Export STEP` became the visible `Export` (aria-label/title still say Export
  STEP), and the source chip/separator padding was tightened.
* The agent card now prints the provider's exact volume/area
  (`35571.416047 mm³`, `11889.101731 mm²`) instead of two decimals.
* `text-overflow: ellipsis` is applied only at `≤1400px`; at 1440 the values are
  unellipsized.

Measured in Chromium 1440×900, FreeCAD design:

| Item | Before client/scroll | After client/scroll |
|---|---|---|
| ENV `60×45×40 mm` | 19 / 59 (truncated) | 59 / 59 |
| VOL | 21 / 65 (truncated) | 51 / 51 |
| AREA | 18 / 65 (truncated) | 51 / 51 |
| SCALE `FIT` | 16 / 16 | 16 / 16 |
| source | `measured by FreeCAD 1.1.4` | `FreeCAD` |
| `barTruncated` | **true** | **false** |

At 1280×800 the same compact values still fit (`barTruncated: false`); the
ellipsis rule is available there if a future longer label needs it.

### 1.5 B4 `solid_count` (generic card rendering)

B4 landed while this cell ran. `DesignAgentCard` already renders checks
generically (`label`, `expected`, `measured`, unit, status). A new test adds a
`solid_count` row (expected 2, measured 2 solid) and asserts the card prints it
with the normal pass mark; no `solid_count`-specific branch exists.

---

## 2. Tests added / updated

New:

| File | Coverage |
|---|---|
| `test/sectionCut.test.ts` (6) | default XZ/4; mid-plane labels from bounds; primitive XY; per-plane defaults; clay corners on the shared plane; store is one state and rejects a bad plane/offset. |

Updated:

| File | Coverage |
|---|---|
| `test/HlrDrawing.dom.test.tsx` | label ↔ basis pin (including the vectors); cut note in the heading outside the SVG; iso+front A–A trace; no trace for a different plane; DOM bounding-box overlap sweep at 1440 and 1280. |
| `test/DraftingSheet.dom.test.tsx` | primitive labels now include `XY`; HLR uses the heading cut note; the sheet label follows a store plane change (XZ/4 ↔ XY/mid-plane). |
| `test/AuxView.dom.test.tsx` | clicking XY writes the shared store cut and the footer prints `CUT A–A · XY · Z = 20.00 mm · mid-plane`. |
| `test/BottomBarDesignReadout.dom.test.tsx` | compact `35 571`/`11 889`, `FreeCAD` chip, full-precision titles, `describeDesignReadout` values. |
| `test/DesignAgentCard.dom.test.tsx` | exact volume/area in the card; generic `solid_count` row. |
| `test/journalPoller.test.ts` | `wake()` reads immediately on a design; wake during an in-flight poll is honoured; the recorded B2 page merges 10 events and verifies. |
| `test/journalLedger.dom.test.tsx` | all ten B2 events render with SEQ/digest/chain `ok`, interleaved with two local rows at their real timestamps. |
| `test/designFixtures.ts` | recorded 10-event B2 journal page (`bracketJournalB2Page`). |
| `test/layoutCss.test.ts` | AuxView tabs take their own row at ≤1400; bottom-bar values have no ellipsis at 1440 and ellipsis only at ≤1400. |

The overlap test reads each label's `x`, `y`, `text-anchor`, and text from the
rendered DOM, builds its box at 1440 and 1280 sheet scales, and asserts every
pair is disjoint. The cut note is asserted to be an HTML heading row, not an SVG
text label, so it cannot collide with the top-view caption.

---

## 3. Gates

Run in `studio/` on the final tree:

| Command | Result |
|---|---|
| `npm run typecheck` | **clean**, exit 0 |
| `npm test` | **43 files, 614 tests, 614 passed** (3.0 s) |
| `npm run build` | **green** (`built in 515ms`); only the pre-existing three.js chunk warning (`559.64 kB / 138.50 kB gzip`) |

`nativeBackendPackaging.test.ts` is green in this run (the parallel Rust
readiness owner fixed its fixture before this cell's final gate; this cell did
not touch it).

---

## 4. Live reproduction and screenshots

### Backend / browser

* `KALA_STUDIO_TOKEN=<throwaway> .venv/bin/python -m kala_agent.studio --data-dir <fresh temp dir> --static-dist studio/dist --port 8782/8785/8786`
* Python backend: `design_available: true`, FreeCAD `1.1.4`, OCP `8.0.1.1`.
* Headless Chromium 154 over CDP, 1440×900 and 1280×800.
* Prompt: *"Design a machined aluminum L-bracket for a 40 mm mounting plate, 8 mm wall thickness, with a 10 mm centered bore and 4x M5 corner mounting holes."*
* FreeCAD target selected in the C2 selector; a second turn with
  **OpenCascade (opt-in)** for comparison.

Recorded results:

```
provider        freecad 1.1.4        volume 35571.416047 mm³
area            11889.101731 mm²     bounds [-30, ~0, 0] … [30, 45, 40]
section         XZ, offset 4.00 mm, area 2,321.46 mm², 1 outer + 1 inner
journal         10 server events (SEQ 1–10), 7 local rows, HASH-CHAIN OK
opencascade     measured by OpenCascade (opt-in) 8.0.1.1
view label      view: OpenCascade import (view-only)   [FreeCAD part]
                view: OpenCascade (opt-in) build       [OpenCascade part]
```

### Before / after shots

| File | Shows |
|---|---|
| `shots/before-1440-pencil.png` | Wrong `TOP`/`FRONT`/`RIGHT` labels; cut note over `TOP · LOOKING −Y`; mid-sheet A–A line; 7 local-only ledger rows; truncated ENV/VOL/AREA. |
| `shots/before-1440-ledger.png` | Same session after the idle tick: server rows eventually appear, proving the race. |
| `shots/before-1440-clay3d.png` | Clay plane at Z = 20 mid-plane while AuxView is XZ at 4. |
| `shots/before-1440-section.png` | Sheet says Z = 20 mid-plane; AuxView says XZ / area 2,321.46. |
| `shots/before-1280-compact.png` | Pre-fix 1280 state. |
| `shots/before-opencascade-1440.png` | Pre-fix OpenCascade comparison. |
| `shots/after-1440-pencil.png` | FreeCAD L-bracket, correct direction labels, A–A trace in iso/front, shared cut label, server ledger rows, full bottom bar. |
| `shots/after-1440-hlr-freecad-closeup.png` | 2× HLR crop: `TOP · LOOKING −Z`, `FRONT · LOOKING +Y`, `RIGHT · LOOKING −X`, `ISO · FROM (+X −Y +Z)`, `CUT A–A · XZ · Y = 4.00 mm`, A flags. |
| `shots/after-1440-bar-closeup.png` | 2× bottom bar: `ENV 60×45×40 mm · VOL 35 571 mm³ · AREA 11 889 mm² · SCALE FIT · FreeCAD`. |
| `shots/after-1440-ledger-top.png` | Ledger scrolled to the top: server `SEQ 1` … with digest and `CHAIN ok`, local rows mixed in. |
| `shots/after-1440-ledger.png` | Ledger at the newest rows. |
| `shots/after-1440-section.png` | Pencil sheet with the AuxView Section panel: `CUT A–A · XZ · Y = 4.00 mm`, area 2,321.46 mm². |
| `shots/after-1440-aux-closeup.png` | 2× AuxView: XZ plane selected, shared cut label, section loop and area. |
| `shots/after-1440-clay3d.png` | Clay viewport with the shared cutting plane. |
| `shots/after-1280-compact.png` | 1280: no label overlap, AuxView tabs wrap the cut note to a second row, bar values fit. |
| `shots/after-opencascade-1440.png` | OpenCascade (opt-in) comparison: `view: OpenCascade (opt-in) build`, `measured by OpenCascade (opt-in) 8.0.1.1`. |
| `shots/after-measurements.json`, `shots/before-measurements.json`, `shots/after-freecad-clean-measurements.json` | Raw DOM measurements used above (bar widths, label rects, ledger rows, traces). |

The before shots were taken from the pre-fix `studio/dist` (verified to still
contain `LOOKING −Y`), with a fresh data dir and the same FreeCAD flow. The
after shots were taken after `npm run build` from the fixed source.

---

## 5. Files changed

New:

* `studio/src/state/sectionCut.ts`
* `studio/test/sectionCut.test.ts`

Modified — source:

* `studio/src/App.tsx` (wake on newest design)
* `studio/src/state/store.ts` (shared `sectionCut`, action, selector, setter)
* `studio/src/state/journalPoller.ts` (`wake()`)
* `studio/src/viewer/hlr.ts` (view bases/captions, section projection, fit helper)
* `studio/src/viewer/HlrDrawing.tsx` (heading cut note, section trace, captions)
* `studio/src/viewer/SceneCanvas.tsx` (shared plane corners)
* `studio/src/components/drafting/DraftingSheet.tsx` (shared cut, section load, note placement)
* `studio/src/components/drafting/AuxView.tsx` (shared plane, footer note)
* `studio/src/components/bottombar/BottomBar.tsx` (compact readout/source, titles)
* `studio/src/components/chat/designCard.ts` (exact values, short provider label)
* `studio/src/styles/drafting.css`, `layout.css`, `bottombar.css`

Modified — tests: `HlrDrawing.dom.test.tsx`, `DraftingSheet.dom.test.tsx`,
`AuxView.dom.test.tsx`, `BottomBarDesignReadout.dom.test.tsx`,
`DesignAgentCard.dom.test.tsx`, `journalPoller.test.ts`,
`journalLedger.dom.test.tsx`, `designFixtures.ts`, `layoutCss.test.ts`.

No Python, Rust, Electron, adapter, or packaging file was edited.

---

## 6. Honest notes / remaining gaps

1. **The `after-1440-pencil.png` and `after-1440-ledger.png` frames are the
   same image.** That is the journal fix: the ledger is already complete when
   the HASH-CHAIN OK stamp appears, so waiting does not add rows. The top-scrolled
   ledger shot is a separate frame.
2. **The OpenCascade comparison title block still ellipsizes**
   `measured by OpenCascade (opt-in) 8.0.1.1` in the narrow DESIGN/MEASURED BY
   cell (full text in the cell title). The bar source chip and the HLR view
   label show the provider without ellipsis. The FreeCAD primary shot prints the
   requested `measured by FreeCAD 1.1.4` in full.
3. **The clay cutting plane is an outline, not a clipped section.** It is drawn
   at the shared plane (XZ y = 4) with a small pad and no depth test, so it
   locates the plane but does not hide the solid. The measured section area and
   loops remain in the AuxView panel, not in the 3D view.
4. **The XZ offset is still fixed at y = 4 mm.** That is ADR criterion 4 and the
   task's shared plane; there is no general offset input. XY/YZ use the
   backend's mid-plane (`offsetMm: null`) and print the resolved value.
5. **If `/section` fails or is not loaded, the HLR sheet has no redline trace**
   (the heading still names the shared plane). It never draws a guessed cut.
6. **At 1280 the AuxView header wraps the cut note onto a second row**, and the
   header is therefore slightly taller. It stays inside the fixed `--aux-height`
   and the body still fits; the measured boxes show no overlap.
7. **`provider.build.requested` has no dedicated journal summary case**; it falls
   back to the generic scalar summary (`design_id=… · provider=… · provider_version=…`).
   It is honest and the event is present, but less polished than the other rows.
8. **No remote provider was called and no credential was read.** All numbers in
   the report and screenshots are from the local Python backend/FreeCAD/OCP
   measurements above, or are the UI's real DOM readings.
