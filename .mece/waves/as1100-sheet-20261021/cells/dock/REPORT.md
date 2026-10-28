# Cell dock (Phase B) — report

The dock's two instruments: the `#6` mini-map (top row, 28%) and the `#5`
graphite process & verification trail (middle row, 48%).

## Gates

| Gate | Result |
|---|---|
| `npm run typecheck` in `studio/` | pass (exit 0) |
| `npm test` in `studio/` | **1184 passed / 1184** in **86 files** (baseline 817 in 59 files at the start of the cell; the other five Phase B cells added the rest) |
| `npm run build` in `studio/` | pass (exit 0; only the pre-existing three.js chunk-size warning) |

Every gate ran with `HOME`, `KALA_AGENT_DATA_DIR`, `KALA_STUDIO_DATA_DIR` and
`XDG_DATA_HOME` pointed at a fresh `mktemp -d` and
`npm_config_cache=/home/v9i/.npm`. `/home/v9i/.local/share/kala-agent` was
never touched, no credential/auth/settings file was read, and no real
`claude`/`codex`/`opencode` CLI was run. Every temp dir was removed.

## Files changed

### Created (owned)

- `studio/src/components/dock/MiniMap.tsx` — the three lenses (part locator,
  assembly fit, section A-A), the redline/amber boxes, the hatched section
  with its measured area and minimum wall.
- `studio/src/components/dock/ProcessTrail.tsx` — the status line, the
  `trail-port` anchor, the milestone dots, the three milestones, the audit
  footer with the ledger.
- `studio/src/components/dock/dockLogic.ts` — the pure rules of the dock (lens
  list, placement offset, placed bounds, minimum wall, script pills, status
  line, milestone dots, stamps, check order). No DOM, no store, no network.
- `studio/src/styles/dock.css` — the two instruments' styles.
- `studio/test/dockLogic.test.ts` (33 tests), `studio/test/MiniMap.dom.test.tsx`
  (20), `studio/test/ProcessTrail.dom.test.tsx` (30).

### Modified (owned)

- `studio/src/styles/journal.css` — the ledger now lives in the process
  trail's audit footer, so its rules are scoped under `.journal-panel-host`
  (the wrapper the trail renders around the panel) instead of the retired
  `.studio-zone-right`. The scope is one class deeper than the shell sketch in
  `layout.css`, so it still wins regardless of the stylesheet order (the
  component imports `journal.css` before `main.tsx` imports `layout.css`).
  The unscoped `.studio-copy-button` rules are unchanged.
- `studio/test/journalLedger.dom.test.tsx` — one assertion updated: it named
  the old `.studio-zone-right …` scope of the MESSAGE rule.

### Modified (not owned, mechanical)

These three files assert the *old stub layout* of the components this cell
replaced, so they had to follow:

- `studio/test/shellStubs.dom.test.tsx` — the two dock stub assertions
  (`data-stub="mini-map"`, `data-stub="process-trail"`) now assert the real
  components (`.mini-map` with its three lens tabs, `.process-trail` with its
  status line and its trail port). The `TitleBlock` and `EngineerCard` stub
  assertions in the same file were also refreshed (the title-block cell
  replaced those stubs with real components): the title block is now matched
  by its `data-title-block` hook and the engineer card's section text is
  matched case-insensitively.
- `studio/test/AgentWorkspace.dom.test.tsx` — "mounts every Phase B instrument
  at its WAVE position" no longer expects `data-stub` on the dock's two
  instruments; it asserts `.dock-slot-mini-map .mini-map` and
  `.dock-slot-process-trail .process-trail` instead.

## Contract consumed

Nothing in the store, the types or `AppShell` changed. The cell uses the
frozen contract as it is:

- `MiniMapLens` (`'locator' | 'fit' | 'section'`) — the store's `miniMapLens`
  and the `miniMap/lens` action.
- `FeatureRef` (`designId`, `view`, `bbox` = minU/minV/maxU/maxV in view-plane
  mm) — `hoveredFeature` / `selectedFeature`.
- `SubTab` (`parentDesignId`, `component`, `status`, `childDesignId`) — the
  assembly-fit lens.
- `DesignResult` (`script`, `measured`, `checks`, `verified`, `artifacts`,
  `material`, `tolerances`, `label`, `provider_version`) — the trail.
- `SectionResponse` (`loops`, `area_mm2`, `outer`/`inner` loops) — the section
  lens and the minimum wall.
- `SectionPlane`, `SectionCut` — the one shared cutting plane; the lens writes
  it through the store's `setSectionPlane`.
- `DrawingResponse` / `DrawingView` — the `/drawing` polylines.
- `JournalEntry`, `HashChainStatus`, `SessionJournalState` — the audit footer.
- `ChatMessage` (`text`, `role`), `ChatState.inFlight`, `lastError`,
  `lastErrorKind`, `turnStartedAt` — the status line.
- `DesignController` — the frozen prop of both components; the mini-map uses
  `loadDrawing` / `loadSection`, the trail keeps the prop for the contract.
- `useAnchorRef('trail-port')` from `components/shell/anchors.ts` — the trail
  port.
- `designCard.ts` (`checkSummary`, `downloadableKind`, `envelopeSize`,
  `formatCheckUnit`, `formatCheckValue`, `measuredByLabel`, `nativeKindLabel`)
  is imported, never edited.
- `getDesignArtifact` + `saveBlob` for the artefact downloads.

### The TOP view's u/v axis mapping (verified and written down)

The mini-map's assembly-fit box is placed in the parent assembly's TOP view,
so the mapping was checked against three sources:

- **Backend** — `src/kala_agent/studio/design/drawing.py`:
  `_VIEW_BASES['top'] = ((0.0, 0.0, 1.0), (1.0, 0.0, 0.0))`, i.e. the viewer
  stands at +Z and the view's horizontal axis is +X; the vertical axis is the
  cross product `viewer × horizontal = (0,0,1) × (1,0,0) = (0,1,0)`, i.e. +Y.
  The local frame handed to the HLR projector is `(u_axis, direction × u_axis)`,
  so the returned polylines are already (X, Y) in millimetres.
- **`studio/src/viewer/hlr.ts`** — `DESIGN_VIEW_BASES.top = { viewer: [0,0,1],
  horizontal: 'X', vertical: 'Y' }`, and `SECTION_VIEW_FOR_PLANE.XY = 'top'`,
  so a TOP view's (u, v) frame is also the XY section's frame.
- **`studio/src/viewer/draftingGeometry.ts`** — the SVG fit flips v
  (`originY + (extent.maxV − v) · scale`), so +Y points up on the sheet.

**Conclusion:** a TOP view's (u, v) pair is the model's (X, Y). The fit box
is therefore `child.measured.bounds` shifted by the parent script's `place`
operation's **x** (u) and **y** (v); the placement's z is the depth the top
view cannot show and is not used for the box. The rule is implemented in
`dockLogic.ts` (`placeOffset`, `placedTopBounds`) and documented in the
header of `MiniMap.tsx` and `dockLogic.ts`.

## What the two instruments show

### Mini-map (`#6`)

- Header micro-tabs `PART LOCATOR`, `ASSEMBLY FIT`, `SECTION A-A`, driven by
  one data array (`MINI_MAP_LENSES` in `dockLogic.ts`); arrow/Home/End
  keyboard navigation; the lens is the store's `miniMapLens`.
- **Part Locator** — with a hovered (or selected) feature: that feature's own
  orthographic view from the design's `/drawing`, fitted, with a vermilion
  (`--accent-redline`) box on the feature's `bbox`, padded by
  `LOCATOR_BOX_PAD_PX = 4` sheet units. Without a feature: the active
  design's TOP view. With only a primitive: the live front/top/right
  silhouettes projected from the real preview bounds, in third angle.
- **Assembly Fit** — the parent design's TOP view ghosted (`--ink-muted`,
  40 % opacity), with an amber (`--accent-amber`) box on the placed bounds of
  the pinned component. Empty state: *"Pin a component from the tab's paper
  roll to see where it fits."* A component that is still building or that
  failed says so.
- **Section A-A** — the shared cutting plane (XY/XZ/YZ switch that writes the
  one shared state), the 45° ANSI hatch, the measured area, the loop counts,
  the offset, and the **MIN WALL** callout. The primitive profile is drawn
  from the live parameters and the real mid-plane.

### Process trail (`#5`)

- **Status line**, derived only from real state:
  `SCRIBBLING GEOMETRY…` (+ the elapsed clock) → `FAILED · <code or kind>` →
  `VERIFIED · MANIFOLD SOLID` → `VERIFIED · n SOLIDS` → `BUILT · CHECKS FAILED
  (k)` → `REPLY · NO GEOMETRY` → `IDLE · READY`.
- **`trail-port`** anchor published at the top of the milestone trail; the
  pencil's graphite string ends there.
- **The dotted trail** `• ── • ── • ── ◉`. A dot is filled only when its
  milestone is really achieved (intent = a compiled script, metrology = the
  backend's verified verdict, artefacts = a stored STEP or native file) and
  the closing ◉ only when the design is verified.
- **Dot 1 Intent & Script** — the latest assistant reply (clamped at 220
  characters with an expand/collapse control), the design label, the real
  parameter pills from the script (`n components · m steps · k pockets`), the
  declared material and tolerances labelled *"declared — not analysed"*, and
  the `</> Script JSON` chip: the popover shows the pretty JSON with a copy
  control and a download of the stored `script` artefact.
- **Dot 2 B-Rep & Metrology** — `✓ B-REP VALID` and `✓ SOLIDS: n` printed
  only from a passed check; a collapsible check list with the failures first
  (the `designCard.ts` formatters); the metrology grid `VOLUME (cm³)`,
  `SURFACE AREA (mm²)`, `ENVELOPE (X × Y × Z mm)` with *"measured by <provider
  version>"*. With only a primitive: today's closed-form estimate of the live
  preview, labelled *"local estimate — not measured"* / *"server estimate —
  not measured"*.
- **Dot 3 CAD Artifacts** — `↓ STEP (.stp)`, `↓ STL (.stl)`, `↓ FCStd` (when
  the provider reported a native file) and `↓ Script`, downloaded through
  `getDesignArtifact` + `saveBlob`; the receipt in the journal carries the
  real byte count and the verified digest; each artefact's digest has a copy
  control; a failed download shows its error.
- **Audit footer** — `▸ Audit hash-chain · N entries · Verified | Chain broken
  @n | Session log`, from the real session journal and the chain verdict.
  Expanding it shows today's `JournalPanel` table, scrollable inside the dock,
  with its poll-error rendering.

## The minimum wall

`minWallMm(loops)` measures the smallest distance between an inner loop and
an outer loop of the section cut, on the kernel's own loop polylines:

- the loops are read as closed chains (whether or not the kernel repeats the
  first point);
- the distance between two segments is the exact segment-to-segment distance
  (crossing segments are zero apart, otherwise the smallest endpoint-to-
  segment distance);
- the wall is the minimum over every inner/outer pair;
- it is `null` — and no callout is printed — when the cut has no inner loop
  (a solid section has no wall) or no outer loop.

## Known gaps

1. **The old `components/drafting/AuxView.tsx` and its test are still in the
   tree.** Its behaviour now lives in the dock, and every behaviour it
   covered is covered by the new tests, so it can be deleted by qa:

   | Old AuxView behaviour | New home | New test |
   |---|---|---|
   | hatched section, measured area, loop counts, `≈` for an approximate area | Mini-map, Section A-A lens | `test/MiniMap.dom.test.tsx` → "the section lens" |
   | plane switch writing the one shared cut | Mini-map, Section A-A lens | "writes the chosen plane to the one shared cutting plane" |
   | section loading state, error state, retry | Mini-map, Section A-A lens | "shows the real error and retries the same plane" |
   | primitive section profile, circle/great circle, unavailable state | Mini-map, Section A-A lens | "draws the primitive profile when no design is on the sheet" |
   | primitive front/top/right silhouettes, "no mesh bounds yet" | Mini-map, Part Locator | "shows the live silhouettes when only a primitive is on the sheet" |
   | declared material and tolerances, "declared — not analysed" | Process trail, Dot 1 | `test/ProcessTrail.dom.test.tsx` → "dot 1 — intent and script" |
   | the ortho 3-view panel | the 2D orthographic sheet (ortho-sheet cell) | `test/OrthoSheet.dom.test.tsx` (ortho-sheet cell) |

2. **The journal's own behavioural tests stay where they were**
   (`test/journalLedger.dom.test.tsx`): the panel is unchanged, it only moved
   into the trail's audit footer. The one edited assertion is the CSS scope.

3. **The `+4 px` redline padding is in the mini-map sheet's own units.** The
   mini-map draws in an SVG `viewBox` of 320 × 176 that is scaled into the
   340 px dock, so one view unit is ≈ 1.06 CSS px. The pad is 4 view units
   (≈ 4.3 CSS px) on every side of the feature's bounding box.

4. **The status line has three honest sub-states the brief did not name**:
   `BUILT · CHECKS NOT REPORTED (k)` when no check failed but the provider
   did not report them, `BUILT · NOT VERIFIED` when the backend reported no
   checks at all, and `VERIFIED · SOLID COUNT NOT REPORTED` when a design
   verifies without a solid count. Printing `CHECKS FAILED (0)` in those
   cases would be a false claim (D1).

5. **The trail does not open the Engineer card.** The relocated table gives
   the CAD-target blocker and the export surface to the title-block cell; the
   trail only downloads the artefacts the backend stored.

6. `components/drafting/AuxView.tsx` still compiles and still imports
   `styles/drafting.css`, whose `.aux-*` rules are canvas-3d's to remove.

## Requests

1. **None that block this cell.** The store, the types and `AppShell` were
   enough: the lens, the feature, the sub-tab, the design, the section, the
   chat state and the session journal all carried what the dock needs.

2. **Suggestion for qa (not a contract change).** `journal.css` is now scoped
   under `.journal-panel-host`. If the ledger is mounted anywhere else, that
   wrapper class has to come with it, or the ledger will fall back to the
   shell sketch in `layout.css`.

3. **Suggestion for the title-block cell.** `test/shellStubs.dom.test.tsx` and
   `test/AgentWorkspace.dom.test.tsx` still carry stub assertions for the
   title block and the engineer card. They were refreshed here so the gate
   would stay green; the title-block cell should own them in the end.
